//! Trusted producer ordering and durable checkpoints outside the ledger worker.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use capsem_foundation::ipc_channel;
use capsem_proto::ledger::{LedgerChannelGrant, LedgerWelcome};
#[cfg(test)]
use capsem_proto::ledger_commitment::ZERO_COMMITMENT_HASH;
use capsem_proto::ledger_commitment::{
    CommitmentClientMessage, CommitmentCommand, CommitmentReply, CommitmentReservation, CommitmentServerMessage,
    LedgerCommitment, MAX_COMMITMENTS_PER_CHECKPOINT,
};
use serde::{Deserialize, Serialize};

use capsem_logger::ledger_protocol::{LedgerQuery, LedgerValue};

const CHECKPOINT_MAGIC: [u8; 4] = *b"CSCP";
const MAX_CHECKPOINT_FRAME_BYTES: usize = 16 * 1024 * 1024;

type Sender = ipc_channel::Sender<CommitmentServerMessage>;
type Receiver = ipc_channel::Receiver<CommitmentClientMessage>;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct ProducerKey {
    generation: [u8; 16],
    client_id: u64,
}

impl ProducerKey {
    fn from_grant(grant: LedgerChannelGrant) -> Self {
        Self {
            generation: grant.generation().as_bytes(),
            client_id: grant.client_id(),
        }
    }

    fn from_commitment(commitment: &LedgerCommitment) -> Self {
        Self {
            generation: commitment.generation().as_bytes(),
            client_id: commitment.client_id(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ProducerTail {
    sequence: u64,
    hash: [u8; 32],
}

#[derive(Default)]
struct State {
    next_global_sequence: u64,
    next_checkpoint_sequence: u64,
    anchored: HashMap<ProducerKey, ProducerTail>,
    pending: HashMap<ProducerKey, Vec<LedgerCommitment>>,
    global_sequences: HashSet<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckpointRecord {
    checkpoint_sequence: u64,
    commitments: Vec<LedgerCommitment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reserved_through: Option<u64>,
}

pub(crate) struct CommitmentAuthority {
    path: PathBuf,
    state: Arc<tokio::sync::Mutex<State>>,
}

/// Fence producer reservations and checkpoints while the ledger snapshot is copied.
pub(crate) struct CheckpointSnapshot {
    path: PathBuf,
    state: tokio::sync::OwnedMutexGuard<State>,
}

impl CheckpointSnapshot {
    async fn append<T: Send + 'static>(
        self,
        record: CheckpointRecord,
        update: impl FnOnce(&mut State) -> T + Send + 'static,
    ) -> Result<T> {
        tokio::task::spawn_blocking(move || {
            // Retain the fence through both disk publication and the memory
            // update, even if the async caller is canceled while we queue.
            append(&self.path, &record)?;
            let mut state = self.state;
            let result = update(&mut state);
            drop(state);
            Ok(result)
        })
        .await
        .context("join ledger commitment append")?
    }

    pub(crate) async fn copy_to(self, session_id: &str) -> Result<()> {
        let source = self.path.clone();
        let root = source
            .parent()
            .and_then(Path::parent)
            .context("checkpoint root missing")?;
        let destination = checkpoint_path(root, session_id);
        if destination == source {
            bail!("cannot fork commitment history onto itself");
        }
        let record = CheckpointRecord {
            checkpoint_sequence: self
                .state
                .next_checkpoint_sequence
                .checked_add(1)
                .context("checkpoint sequence exhausted")?,
            commitments: Vec::new(),
            // A ledger snapshot flushes accepted rows whose producers may not
            // yet have checkpointed. Never reuse any trusted reservation, and
            // never derive this counter from the worker-controlled database.
            reserved_through: Some(self.state.next_global_sequence),
        };
        tokio::task::spawn_blocking(move || {
            use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
            use std::ffi::OsStr;
            // Blocking work outlives a canceled caller. Keep the fence with it.
            let _fence = self.state;
            let root = ContainedDir::open_root(
                source
                    .parent()
                    .and_then(Path::parent)
                    .context("checkpoint root missing")?,
            )?;
            let parent = root.descend(
                source
                    .parent()
                    .and_then(Path::file_name)
                    .context("checkpoint parent missing")?,
            )?;
            let file = parent.open_file(OsStr::new("checkpoints.log"), ContainedOpenOptions::read_only())?;
            let target = root.descend_or_create(
                destination
                    .parent()
                    .and_then(Path::file_name)
                    .context("fork checkpoint parent missing")?,
                0o700,
            )?;
            let (copied, _) =
                capsem_foundation::unix::fs::clone_file_into(&file, &target, OsStr::new("checkpoints.log"), 0o600)?;
            drop(copied);
            let result = (|| {
                let mut file = target.open_existing_private_append(OsStr::new("checkpoints.log"))?;
                append_record(&mut file, &record)?;
                file.sync_all()?;
                target.sync()?;
                root.sync()?;
                Ok(())
            })();
            if result.is_err() {
                let _ = target.remove_private_file(OsStr::new("checkpoints.log"));
            }
            result
        })
        .await
        .context("join trusted fork checkpoint copy")?
    }
}

fn checkpoint_path(root: &Path, session_id: &str) -> PathBuf {
    root.join(capsem_crypto::blake3::hash(session_id.as_bytes()).to_hex().to_string())
        .join("checkpoints.log")
}

impl CommitmentAuthority {
    pub(crate) async fn open(root: &Path, session_id: &str) -> Result<Arc<Self>> {
        let path = checkpoint_path(root, session_id);
        let loaded_path = path.clone();
        let state = tokio::task::spawn_blocking(move || load(&loaded_path))
            .await
            .context("join ledger commitment checkpoint load")??;
        Ok(Arc::new(Self {
            path,
            state: Arc::new(tokio::sync::Mutex::new(state)),
        }))
    }

    pub(crate) async fn snapshot(&self) -> CheckpointSnapshot {
        CheckpointSnapshot {
            path: self.path.clone(),
            state: Arc::clone(&self.state).lock_owned().await,
        }
    }

    async fn reserve(
        &self,
        grant: LedgerChannelGrant,
        producer_sequence: u64,
        event_kind: String,
        event_hash: [u8; 32],
        previous_hash: [u8; 32],
    ) -> Result<u64> {
        let reservation = CommitmentReservation::new(event_kind, event_hash)?;
        let mut sequences = self
            .reserve_batch(grant, producer_sequence, previous_hash, vec![reservation])
            .await?;
        Ok(sequences.pop().expect("one reservation returns one sequence"))
    }

    async fn reserve_batch(
        &self,
        grant: LedgerChannelGrant,
        first_producer_sequence: u64,
        previous_hash: [u8; 32],
        reservations: Vec<CommitmentReservation>,
    ) -> Result<Vec<u64>> {
        if reservations.is_empty() || reservations.len() > capsem_proto::LEDGER_ADMISSION_BATCH_CAPACITY {
            bail!("commitment reservation batch is empty or exceeds its bound");
        }
        for reservation in &reservations {
            reservation.validate()?;
        }
        let key = ProducerKey::from_grant(grant);
        let snapshot = self.snapshot().await;
        let state: &State = &snapshot.state;
        let tail = state.pending.get(&key).and_then(|pending| pending.last()).map_or_else(
            || state.anchored.get(&key).copied().unwrap_or_default(),
            |commitment| ProducerTail {
                sequence: commitment.producer_sequence(),
                hash: commitment.commitment_hash(),
            },
        );
        if first_producer_sequence != tail.sequence.checked_add(1).context("producer sequence exhausted")?
            || previous_hash != tail.hash
        {
            bail!("producer commitment does not extend its trusted tail");
        }
        let mut producer_sequence = first_producer_sequence;
        let mut global_sequence = state.next_global_sequence;
        let mut previous_hash = previous_hash;
        let mut commitments = Vec::with_capacity(reservations.len());
        let mut global_sequences = Vec::with_capacity(reservations.len());
        for reservation in reservations {
            global_sequence = global_sequence
                .checked_add(1)
                .context("global commitment sequence exhausted")?;
            let commitment = LedgerCommitment::new(
                grant,
                producer_sequence,
                global_sequence,
                reservation.event_kind(),
                reservation.event_hash(),
                previous_hash,
            )?;
            previous_hash = commitment.commitment_hash();
            commitments.push(commitment);
            global_sequences.push(global_sequence);
            producer_sequence = producer_sequence
                .checked_add(1)
                .context("producer sequence exhausted")?;
        }
        let checkpoint_sequence = state
            .next_checkpoint_sequence
            .checked_add(1)
            .context("checkpoint sequence exhausted")?;
        let record = CheckpointRecord {
            checkpoint_sequence,
            commitments: Vec::new(),
            // Rows may become durable before their producer anchors them.
            // Persist the allocation, without trusting those rows, before
            // allowing a producer to send them to the ledger worker.
            reserved_through: Some(global_sequence),
        };
        snapshot
            .append(record, move |state| {
                state.next_checkpoint_sequence = checkpoint_sequence;
                state.next_global_sequence = global_sequence;
                state.global_sequences.extend(global_sequences.iter().copied());
                state.pending.entry(key).or_default().extend(commitments);
                global_sequences
            })
            .await
    }

    async fn cancel(&self, grant: LedgerChannelGrant, global_sequence: u64) -> Result<()> {
        let key = ProducerKey::from_grant(grant);
        let mut state = self.state.lock().await;
        let pending = state
            .pending
            .get_mut(&key)
            .context("unknown producer commitment reservation")?;
        if pending.last().map(LedgerCommitment::global_sequence) != Some(global_sequence) {
            bail!("only the producer's unanchored tail may be canceled");
        }
        pending.pop();
        if pending.is_empty() {
            state.pending.remove(&key);
        }
        state.global_sequences.remove(&global_sequence);
        drop(state);
        Ok(())
    }

    async fn anchor(&self, grant: LedgerChannelGrant, commitments: Vec<LedgerCommitment>) -> Result<u64> {
        if commitments.is_empty() || commitments.len() > MAX_COMMITMENTS_PER_CHECKPOINT {
            bail!("commitment checkpoint batch is empty or exceeds its bound");
        }
        let key = ProducerKey::from_grant(grant);
        let snapshot = self.snapshot().await;
        let state: &State = &snapshot.state;
        let pending = state
            .pending
            .get(&key)
            .context("producer has no reserved commitments")?;
        if commitments.len() > pending.len() || pending[..commitments.len()] != commitments {
            bail!("checkpoint does not match the producer's reserved prefix");
        }
        for commitment in &commitments {
            commitment.validate_grant(grant)?;
        }
        let checkpoint_sequence = state
            .next_checkpoint_sequence
            .checked_add(1)
            .context("checkpoint sequence exhausted")?;
        let record = CheckpointRecord {
            checkpoint_sequence,
            commitments: commitments.clone(),
            reserved_through: None,
        };
        snapshot
            .append(record, move |state| {
                let tail = commitments.last().expect("nonempty commitments checked");
                state.anchored.insert(
                    key,
                    ProducerTail {
                        sequence: tail.producer_sequence(),
                        hash: tail.commitment_hash(),
                    },
                );
                let pending = state.pending.get_mut(&key).expect("pending prefix checked");
                pending.drain(..commitments.len());
                if pending.is_empty() {
                    state.pending.remove(&key);
                }
                state.next_checkpoint_sequence = checkpoint_sequence;
                checkpoint_sequence
            })
            .await
    }

    async fn abandon(&self, grant: LedgerChannelGrant) {
        let key = ProducerKey::from_grant(grant);
        let mut state = self.state.lock().await;
        if let Some(pending) = state.pending.remove(&key) {
            for commitment in pending {
                state.global_sequences.remove(&commitment.global_sequence());
            }
        }
    }

    #[cfg(test)]
    async fn anchored(&self) -> Vec<LedgerCommitment> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || read_records(&path).unwrap())
            .await
            .unwrap()
            .into_iter()
            .flat_map(|record| record.commitments)
            .collect()
    }

    async fn checkpointed(&self) -> Result<Vec<LedgerCommitment>> {
        let path = self.path.clone();
        Ok(tokio::task::spawn_blocking(move || read_records(&path))
            .await
            .context("join ledger commitment checkpoint read")??
            .into_iter()
            .flat_map(|record| record.commitments)
            .collect())
    }

    pub(crate) async fn verify_ledger(&self, client: &capsem_logger::ledger_client::LedgerClient) -> Result<()> {
        let expected = self.checkpointed().await?;
        let mut actual = HashMap::new();
        let mut after = 0_u64;
        loop {
            let sets = client
                .query(LedgerQuery::Commitments {
                    after_global_sequence: after,
                    limit: 2_000,
                })
                .await
                .map_err(anyhow::Error::msg)?;
            let rows = sets
                .into_iter()
                .next()
                .context("ledger commitment query returned no result set")?;
            let count = rows.rows.len();
            for row in rows.rows {
                let commitment = decode_commitment_row(&row)?;
                if commitment.global_sequence() <= after {
                    bail!("ledger commitment rows are not in trusted global order");
                }
                after = commitment.global_sequence();
                if actual.insert(after, commitment).is_some() {
                    bail!("ledger repeats a commitment global sequence");
                }
            }
            if count < 2_000 {
                break;
            }
        }
        for commitment in expected {
            match actual.get(&commitment.global_sequence()) {
                Some(stored) if stored == &commitment => {}
                Some(_) => bail!(
                    "ledger commitment {} was substituted or altered",
                    commitment.global_sequence()
                ),
                None => bail!("ledger omitted anchored commitment {}", commitment.global_sequence()),
            }
        }
        Ok(())
    }
}

fn decode_commitment_row(row: &[LedgerValue]) -> Result<LedgerCommitment> {
    let [LedgerValue::Integer(global_sequence), LedgerValue::Text(generation), LedgerValue::Integer(client_id), LedgerValue::Text(role), LedgerValue::Integer(producer_sequence), LedgerValue::Text(event_kind), LedgerValue::Text(event_hash), LedgerValue::Text(previous_hash), LedgerValue::Text(commitment_hash)] =
        row
    else {
        bail!("ledger commitment query returned an invalid typed row");
    };
    let generation = hex_array::<16>(generation)?;
    let role = match role.as_str() {
        "vm_owner" => capsem_proto::ledger::LedgerClientRole::VmOwner,
        "proxy" => capsem_proto::ledger::LedgerClientRole::Proxy,
        "coordinator" => capsem_proto::ledger::LedgerClientRole::Coordinator,
        _ => bail!("ledger commitment row has an invalid producer role"),
    };
    let grant = LedgerChannelGrant::new(
        capsem_proto::ledger::LedgerGeneration::new(generation),
        u64::try_from(*client_id).context("ledger commitment client id is negative")?,
        role,
    )?;
    let commitment = LedgerCommitment::new(
        grant,
        u64::try_from(*producer_sequence).context("ledger producer sequence is negative")?,
        u64::try_from(*global_sequence).context("ledger global sequence is negative")?,
        event_kind.clone(),
        hex_array(event_hash)?,
        hex_array(previous_hash)?,
    )?;
    if commitment.commitment_hash() != hex_array::<32>(commitment_hash)? {
        bail!("ledger commitment row hash does not match its fields");
    }
    Ok(commitment)
}

fn hex_array<const N: usize>(value: &str) -> Result<[u8; N]> {
    if value.len() != N * 2 {
        bail!("ledger commitment hex field has the wrong length");
    }
    let mut bytes = [0_u8; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .context("ledger commitment hex field is invalid")?;
    }
    Ok(bytes)
}

pub(crate) async fn serve(
    authority: Arc<CommitmentAuthority>,
    stream: UnixStream,
    grant: LedgerChannelGrant,
) -> Result<()> {
    let (sender, receiver) = ipc_channel::channel_from_std::<CommitmentServerMessage, CommitmentClientMessage>(stream)?;
    let first = receiver.recv().await?;
    let CommitmentClientMessage::Hello { hello } = first else {
        bail!("commitment client sent a request before hello");
    };
    grant.validate_hello(&hello)?;
    sender
        .send(CommitmentServerMessage::Welcome {
            welcome: LedgerWelcome::for_grant(&grant),
        })
        .await?;

    let outcome = serve_requests(&authority, &sender, &receiver, grant).await;
    authority.abandon(grant).await;
    outcome
}

async fn serve_requests(
    authority: &CommitmentAuthority,
    sender: &Sender,
    receiver: &Receiver,
    grant: LedgerChannelGrant,
) -> Result<()> {
    loop {
        let CommitmentClientMessage::Request { request_id, command } = receiver.recv().await? else {
            bail!("commitment client repeated hello");
        };
        if request_id == 0 {
            bail!("commitment request id must not be zero");
        }
        let result = match command {
            CommitmentCommand::Reserve {
                producer_sequence,
                event_kind,
                event_hash,
                previous_hash,
            } => authority
                .reserve(grant, producer_sequence, event_kind, event_hash, previous_hash)
                .await
                .map(|global_sequence| CommitmentReply::Reserved { global_sequence }),
            CommitmentCommand::ReserveBatch {
                first_producer_sequence,
                previous_hash,
                reservations,
            } => authority
                .reserve_batch(grant, first_producer_sequence, previous_hash, reservations)
                .await
                .map(|global_sequences| CommitmentReply::ReservedBatch { global_sequences }),
            CommitmentCommand::Cancel { global_sequence } => authority
                .cancel(grant, global_sequence)
                .await
                .map(|()| CommitmentReply::Canceled),
            CommitmentCommand::Anchor { commitments } => authority
                .anchor(grant, commitments)
                .await
                .map(|checkpoint_sequence| CommitmentReply::Anchored { checkpoint_sequence }),
        };
        let reply = result.unwrap_or_else(|error| CommitmentReply::Failed {
            message: format!("{error:#}"),
        });
        sender
            .send(CommitmentServerMessage::Response { request_id, reply })
            .await?;
    }
}

fn load(path: &Path) -> Result<State> {
    initialize(path)?;
    let records = read_records(path)?;
    let mut state = State::default();
    for record in records {
        if record.checkpoint_sequence != state.next_checkpoint_sequence + 1 {
            bail!("ledger commitment checkpoint sequence is not contiguous");
        }
        if let Some(through) = record.reserved_through {
            if !record.commitments.is_empty() || through < state.next_global_sequence {
                bail!("fork commitment reservation boundary is invalid");
            }
            state.next_global_sequence = through;
            state.next_checkpoint_sequence = record.checkpoint_sequence;
            continue;
        }
        if record.commitments.is_empty() || record.commitments.len() > MAX_COMMITMENTS_PER_CHECKPOINT {
            bail!("ledger commitment checkpoint has an invalid batch size");
        }
        for commitment in record.commitments {
            commitment.validate()?;
            let key = ProducerKey::from_commitment(&commitment);
            let tail = state.anchored.get(&key).copied().unwrap_or_default();
            if commitment.producer_sequence() != tail.sequence + 1 || commitment.previous_hash() != tail.hash {
                bail!("ledger commitment checkpoint breaks a producer chain");
            }
            if !state.global_sequences.insert(commitment.global_sequence()) {
                bail!("ledger commitment checkpoint repeats a global sequence");
            }
            state.next_global_sequence = state.next_global_sequence.max(commitment.global_sequence());
            state.anchored.insert(
                key,
                ProducerTail {
                    sequence: commitment.producer_sequence(),
                    hash: commitment.commitment_hash(),
                },
            );
        }
        state.next_checkpoint_sequence = record.checkpoint_sequence;
    }
    Ok(state)
}

fn initialize(path: &Path) -> Result<()> {
    let parent = path.parent().context("commitment checkpoint path has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    file.sync_all().with_context(|| format!("sync {}", path.display()))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("sync {}", parent.display()))
}

fn append(path: &Path, record: &CheckpointRecord) -> Result<()> {
    let mut file = OpenOptions::new().append(true).open(path)?;
    append_record(&mut file, record)
}

fn append_record(file: &mut File, record: &CheckpointRecord) -> Result<()> {
    let payload = rmp_serde::to_vec_named(record).context("encode commitment checkpoint")?;
    if payload.len() > MAX_CHECKPOINT_FRAME_BYTES {
        bail!("encoded commitment checkpoint exceeds its bound");
    }
    file.write_all(&CHECKPOINT_MAGIC)?;
    file.write_all(&(payload.len() as u32).to_be_bytes())?;
    file.write_all(&payload)?;
    file.write_all(capsem_crypto::blake3::hash(&payload).as_bytes())?;
    file.sync_data()?;
    Ok(())
}

fn read_records(path: &Path) -> Result<Vec<CheckpointRecord>> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut records = Vec::new();
    let mut valid_end = 0_u64;
    loop {
        let mut header = [0_u8; 8];
        match file.read(&mut header[..1])? {
            0 => break,
            1 => {}
            _ => unreachable!(),
        }
        if let Err(error) = file.read_exact(&mut header[1..]) {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                file.set_len(valid_end)?;
                break;
            }
            return Err(error.into());
        }
        if header[..4] != CHECKPOINT_MAGIC {
            bail!("ledger commitment checkpoint magic is corrupt");
        }
        let length = u32::from_be_bytes(header[4..].try_into().expect("four-byte length")) as usize;
        if length == 0 || length > MAX_CHECKPOINT_FRAME_BYTES {
            bail!("ledger commitment checkpoint frame length is invalid");
        }
        let mut payload = vec![0; length];
        let mut checksum = [0_u8; 32];
        if let Err(error) = file
            .read_exact(&mut payload)
            .and_then(|()| file.read_exact(&mut checksum))
        {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                file.set_len(valid_end)?;
                break;
            }
            return Err(error.into());
        }
        if checksum != *capsem_crypto::blake3::hash(&payload).as_bytes() {
            bail!("ledger commitment checkpoint checksum is corrupt");
        }
        records.push(rmp_serde::from_slice(&payload).context("decode commitment checkpoint")?);
        valid_end = file.stream_position()?;
    }
    file.seek(SeekFrom::End(0))?;
    Ok(records)
}

#[cfg(test)]
mod tests;
