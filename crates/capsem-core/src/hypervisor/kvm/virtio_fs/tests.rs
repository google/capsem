use super::*;
use crate::hypervisor::fuse::file_handles::FileHandleKindSnapshot;
use crate::hypervisor::kvm::memory::{GuestMemory, RAM_BASE};
use crate::hypervisor::kvm::virtio_queue::VirtqDesc;
use std::collections::BTreeMap;
use std::io::{Seek, SeekFrom};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, OnceLock};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context as LayerContext;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Layer;
mod descriptor_bounds;
static EVENT_CAPTURE_LOCK: Mutex<()> = Mutex::new(());
static EVENT_CAPTURE: OnceLock<EventCaptureState> = OnceLock::new();

struct EventCaptureState {
    dispatcher: tracing::Dispatch,
    _interest_guard: tracing::Dispatch,
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Clone, Default)]
struct EventCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Debug)]
struct CapturedEvent {
    level: Level,
    fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldCapture(BTreeMap<String, String>);

impl Visit for FieldCapture {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

impl<S> Layer<S> for EventCapture
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, S>) {
        let mut fields = FieldCapture::default();
        event.record(&mut fields);
        self.events.lock().unwrap().push(CapturedEvent {
            level: *event.metadata().level(),
            fields: fields.0,
        });
    }
}

fn capture_events(run: impl FnOnce()) -> Vec<CapturedEvent> {
    let _capture_guard = EVENT_CAPTURE_LOCK.lock().unwrap();
    let state = EVENT_CAPTURE.get_or_init(|| {
        let capture = EventCapture::default();
        let events = Arc::clone(&capture.events);
        let dispatcher = tracing::Dispatch::new(tracing_subscriber::registry().with(capture));
        let interest_guard = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        EventCaptureState {
            dispatcher,
            _interest_guard: interest_guard,
            events,
        }
    });

    // Tracing's callsite interest cache is process-global. Keeping two
    // dispatchers registered for the parallel test process forces tracing-core
    // to consult its dispatcher registry instead of whichever thread first
    // registers a callsite. The capture dispatcher's interest therefore cannot
    // be replaced by an unrelated test thread's no-subscriber default.
    state.events.lock().unwrap().clear();
    tracing::dispatcher::with_default(&state.dispatcher, run);
    std::mem::take(&mut *state.events.lock().unwrap())
}

fn events_named<'a>(events: &'a [CapturedEvent], name: &str) -> Vec<&'a CapturedEvent> {
    events
        .iter()
        .filter(|event| event.fields.get("event_name").map(String::as_str) == Some(name))
        .collect()
}

fn emit_capture_probe() {
    debug!(event_name = "virtio.fs.capture_probe", "structured capture probe");
}

pub(super) fn temp_share(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("capsem-virtfs-test").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn host_file_type(metadata: &std::fs::Metadata) -> u32 {
    const MASK: u32 = libc::S_IFMT as _;
    metadata.mode() & MASK
}

/// Helper: create a FuseProcessor for testing (no queues needed).
pub(super) fn test_processor(dir: &Path) -> FuseProcessor {
    FuseProcessor {
        root_path: dir.to_path_buf(),
        read_only: false,
        inodes: InodeTable::new(dir).unwrap(),
        file_handles: FileHandleTable::new(),
    }
}

fn test_device(dir: &Path) -> VirtioFsDevice {
    VirtioFsDevice::new("capsem", dir, false, -1, Arc::new(AtomicU32::new(0))).unwrap()
}

const TEST_QUEUE_SIZE: u16 = 8;

struct WorkerHarness {
    mem: GuestMemory,
    hiprio: VirtQueue,
    request: VirtQueue,
    irq_fd: OwnedFd,
    interrupt_status: Arc<AtomicU32>,
}

fn worker_harness() -> WorkerHarness {
    let mem = GuestMemory::new(1024 * 1024).unwrap();
    let memref = mem.clone_ref(RAM_BASE);
    let hiprio = VirtQueue::new(
        memref.clone(),
        RAM_BASE,
        RAM_BASE + 0x100,
        RAM_BASE + 0x200,
        TEST_QUEUE_SIZE,
    );
    let request = VirtQueue::new(
        memref,
        RAM_BASE + 0x400,
        RAM_BASE + 0x500,
        RAM_BASE + 0x600,
        TEST_QUEUE_SIZE,
    );
    let raw_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    assert!(raw_fd >= 0);
    WorkerHarness {
        mem,
        hiprio,
        request,
        irq_fd: unsafe { OwnedFd::from_raw_fd(raw_fd) },
        interrupt_status: Arc::new(AtomicU32::new(0)),
    }
}

fn write_test_descriptor(mem: &GuestMemory, desc: VirtqDesc) {
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&desc.addr.to_le_bytes());
    bytes[8..12].copy_from_slice(&desc.len.to_le_bytes());
    bytes[12..14].copy_from_slice(&desc.flags.to_le_bytes());
    bytes[14..16].copy_from_slice(&desc.next.to_le_bytes());
    mem.write_at(0, &bytes).unwrap();
}

fn run_worker_notification(harness: WorkerHarness, dir: &Path, queue_index: u32) -> WorkerHarness {
    let WorkerHarness {
        mem,
        hiprio,
        request,
        irq_fd,
        interrupt_status,
    } = harness;
    let (tx, rx) = mpsc::channel();
    let proc = test_processor(dir);
    let memref = mem.clone_ref(RAM_BASE);
    let irq_raw_fd = irq_fd.as_raw_fd();
    let worker_status = Arc::clone(&interrupt_status);
    let dispatcher = tracing::dispatcher::get_default(Clone::clone);
    let handle = std::thread::spawn(move || {
        tracing::dispatcher::with_default(&dispatcher, || {
            worker_loop(proc, request, hiprio, memref, rx, irq_raw_fd, worker_status)
        })
    });

    tx.send(WorkerCommand::Notify(queue_index)).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    tx.send(WorkerCommand::Checkpoint {
        tag: [0; TAG_LEN],
        done: done_tx,
    })
    .unwrap();
    done_rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    drop(tx);
    handle.join().unwrap();

    let memref = mem.clone_ref(RAM_BASE);
    WorkerHarness {
        mem,
        hiprio: VirtQueue::new(
            memref.clone(),
            RAM_BASE,
            RAM_BASE + 0x100,
            RAM_BASE + 0x200,
            TEST_QUEUE_SIZE,
        ),
        request: VirtQueue::new(
            memref,
            RAM_BASE + 0x400,
            RAM_BASE + 0x500,
            RAM_BASE + 0x600,
            TEST_QUEUE_SIZE,
        ),
        irq_fd,
        interrupt_status,
    }
}

fn assert_irq_not_signaled(harness: &WorkerHarness) {
    assert_eq!(harness.interrupt_status.load(Ordering::SeqCst), 0);
    let mut value = 0u64;
    let ret = unsafe {
        libc::read(
            harness.irq_fd.as_raw_fd(),
            &mut value as *mut u64 as *mut libc::c_void,
            std::mem::size_of::<u64>(),
        )
    };
    assert_eq!(ret, -1);
    assert_eq!(std::io::Error::last_os_error().kind(), std::io::ErrorKind::WouldBlock);
}

fn assert_irq_signaled(harness: &WorkerHarness) {
    assert_eq!(harness.interrupt_status.load(Ordering::SeqCst), 1);
    let mut value = 0u64;
    let ret = unsafe {
        libc::read(
            harness.irq_fd.as_raw_fd(),
            &mut value as *mut u64 as *mut libc::c_void,
            std::mem::size_of::<u64>(),
        )
    };
    assert_eq!(ret, std::mem::size_of::<u64>() as isize);
    assert_eq!(value, 1);
}

fn enqueue_hiprio_request(harness: &WorkerHarness, avail_flags: u16) {
    let header = make_header(0, 1, 1);
    let request = fuse::as_bytes(&header);
    harness.mem.write_at(0x1000, request).unwrap();
    write_test_descriptor(
        &harness.mem,
        VirtqDesc {
            addr: RAM_BASE + 0x1000,
            len: request.len() as u32,
            flags: 0,
            next: 0,
        },
    );
    harness.mem.write_at(0x100, &avail_flags.to_le_bytes()).unwrap();
    harness.mem.write_at(0x102, &1u16.to_le_bytes()).unwrap();
    harness.mem.write_at(0x104, &0u16.to_le_bytes()).unwrap();
}

#[test]
fn empty_queue_notification_does_not_raise_irq() {
    let dir = temp_share("empty-notify");
    let harness = run_worker_notification(worker_harness(), &dir, 1);
    assert_irq_not_signaled(&harness);
}

#[test]
fn completed_queue_honors_driver_interrupt_suppression() {
    let dir = temp_share("suppressed-notify");
    let harness = worker_harness();
    enqueue_hiprio_request(&harness, 1);

    let harness = run_worker_notification(harness, &dir, 0);
    let mut used_idx = [0u8; 2];
    harness.mem.read_at(0x202, &mut used_idx).unwrap();
    assert_eq!(u16::from_le_bytes(used_idx), 1);
    assert_irq_not_signaled(&harness);
}

#[test]
fn completed_queue_raises_irq_when_driver_requests_it() {
    let dir = temp_share("interrupt-notify");
    let harness = worker_harness();
    enqueue_hiprio_request(&harness, 0);

    let harness = run_worker_notification(harness, &dir, 0);
    let mut used_idx = [0u8; 2];
    harness.mem.read_at(0x202, &mut used_idx).unwrap();
    assert_eq!(u16::from_le_bytes(used_idx), 1);
    assert_irq_signaled(&harness);
}

#[test]
fn queue_notification_is_trace_only() {
    let dir = temp_share("queue-notify-log-level");
    let events = capture_events(|| {
        test_device(&dir).queue_notify(1);
    });
    let notify = events_named(&events, "virtio.fs.queue_notify");

    assert_eq!(notify.len(), 1, "{events:#?}");
    assert_eq!(notify[0].level, Level::TRACE);
    assert_eq!(notify[0].fields.get("queue_index").unwrap(), "1");
}

#[test]
fn structured_capture_keeps_interest_for_callsite_registered_on_parallel_thread() {
    capture_events(|| {});
    std::thread::spawn(emit_capture_probe).join().unwrap();

    let events = capture_events(emit_capture_probe);
    let probes = events_named(&events, "virtio.fs.capture_probe");

    assert_eq!(probes.len(), 1, "{events:#?}");
    assert_eq!(probes[0].level, Level::DEBUG);
}

#[test]
fn empty_notification_and_checkpoint_keep_only_structured_evidence() {
    let dir = temp_share("empty-notify-structured-log");
    let events = capture_events(|| {
        run_worker_notification(worker_harness(), &dir, 1);
    });
    let drains = events_named(&events, "virtio.fs.queue_drain");
    let quiesce = events_named(&events, "virtio.fs.quiesce");

    assert_eq!(drains.len(), 1, "{events:#?}");
    assert_eq!(drains[0].level, Level::TRACE);
    assert_eq!(drains[0].fields.get("queue").unwrap(), "request");
    assert_eq!(drains[0].fields.get("processed").unwrap(), "0");
    assert_eq!(drains[0].fields.get("should_interrupt").unwrap(), "false");
    assert_eq!(quiesce.len(), 1, "{events:#?}");
    assert_eq!(quiesce[0].level, Level::DEBUG);
    assert_eq!(quiesce[0].fields.get("hiprio_processed").unwrap(), "0");
    assert_eq!(quiesce[0].fields.get("request_processed").unwrap(), "0");
    assert_eq!(quiesce[0].fields.get("hiprio_processed_total").unwrap(), "0");
    assert_eq!(quiesce[0].fields.get("request_processed_total").unwrap(), "0");
    assert_eq!(quiesce[0].fields.get("hiprio_should_interrupt").unwrap(), "false");
    assert_eq!(quiesce[0].fields.get("request_should_interrupt").unwrap(), "false");
}

#[test]
fn nonempty_notification_is_trace_and_aggregated_at_quiesce() {
    let dir = temp_share("nonempty-notify-structured-log");
    let harness = worker_harness();
    enqueue_hiprio_request(&harness, 0);
    let events = capture_events(|| {
        run_worker_notification(harness, &dir, 0);
    });
    let drains = events_named(&events, "virtio.fs.queue_drain");
    let quiesce = events_named(&events, "virtio.fs.quiesce");

    assert_eq!(drains.len(), 1, "{events:#?}");
    assert_eq!(drains[0].level, Level::TRACE);
    assert_eq!(drains[0].fields.get("queue").unwrap(), "hiprio");
    assert_eq!(drains[0].fields.get("processed").unwrap(), "1");
    assert_eq!(drains[0].fields.get("should_interrupt").unwrap(), "true");
    assert_eq!(quiesce.len(), 1, "{events:#?}");
    assert_eq!(quiesce[0].level, Level::DEBUG);
    assert_eq!(quiesce[0].fields.get("hiprio_processed_total").unwrap(), "1");
    assert_eq!(quiesce[0].fields.get("request_processed_total").unwrap(), "0");
}

#[test]
fn closed_worker_notification_warns_once_and_disables_sender() {
    let dir = temp_share("closed-worker-notify-log");
    let mut device = test_device(&dir);
    let (tx, rx) = mpsc::channel();
    device.notify_tx = Some(tx);
    drop(rx);
    let events = capture_events(|| {
        device.queue_notify(1);
        device.queue_notify(1);
    });
    let failures = events_named(&events, "virtio.fs.queue_notify_failed");

    assert_eq!(failures.len(), 1, "{events:#?}");
    assert_eq!(failures[0].level, Level::WARN);
    assert_eq!(failures[0].fields.get("queue_index").unwrap(), "1");
    assert!(device.notify_tx.is_none());
}

#[test]
fn fs_device_type() {
    let dir = temp_share("dev-type");
    assert_eq!(test_device(&dir).device_type(), VIRTIO_ID_FS);
}

#[test]
fn fs_features() {
    let dir = temp_share("features");
    assert_ne!(test_device(&dir).features() & VIRTIO_F_VERSION_1, 0);
}

#[test]
fn fs_two_queues() {
    let dir = temp_share("queues");
    assert_eq!(test_device(&dir).queue_max_sizes(), &[QUEUE_SIZE, QUEUE_SIZE]);
}

#[test]
fn fs_config_tag() {
    let dir = temp_share("cfg-tag");
    let dev = test_device(&dir);
    let mut data = [0u8; 36];
    dev.read_config(0, &mut data);
    assert_eq!(&data[..6], b"capsem");
    assert!(data[6..].iter().all(|&b| b == 0));
}

#[test]
fn fs_config_nrq() {
    let dir = temp_share("cfg-nrq");
    let dev = test_device(&dir);
    let mut data = [0u8; 4];
    dev.read_config(36, &mut data);
    assert_eq!(u32::from_le_bytes(data), 1);
}

#[test]
fn fs_config_past_end() {
    let dir = temp_share("cfg-past");
    let dev = test_device(&dir);
    let mut data = [0xFFu8; 4];
    dev.read_config(40, &mut data);
    assert!(data.iter().all(|&b| b == 0));
}

#[test]
fn init_response_version() {
    let dir = temp_share("init-ver");
    let mut proc = test_processor(&dir);
    let header = FuseInHeader {
        len: 56,
        opcode: FUSE_INIT,
        unique: 1,
        nodeid: 0,
        uid: 0,
        gid: 0,
        pid: 0,
        padding: 0,
    };
    let init_in = FuseInitIn {
        major: 7,
        minor: 38,
        max_readahead: 131072,
        flags: 0,
    };
    let mut req = fuse::as_bytes(&header).to_vec();
    req.extend_from_slice(fuse::as_bytes(&init_in));

    let resp = proc.handle_request(&req);
    let out: FuseOutHeader = fuse::read_struct(&resp).unwrap();
    assert_eq!(out.error, 0);
    let init_out: FuseInitOut = fuse::read_struct(&resp[16..]).unwrap();
    assert_eq!(init_out.major, 7);
    assert_eq!(init_out.minor, 31);
    assert!(init_out.max_write > 0);
}

#[test]
fn init_response_advertises_large_request_pages() {
    let dir = temp_share("init-pages");
    let mut proc = test_processor(&dir);
    let header = FuseInHeader {
        len: 56,
        opcode: FUSE_INIT,
        unique: 2,
        nodeid: 0,
        uid: 0,
        gid: 0,
        pid: 0,
        padding: 0,
    };
    let init_in = FuseInitIn {
        major: 7,
        minor: 38,
        max_readahead: 1024 * 1024,
        flags: FUSE_BIG_WRITES | FUSE_MAX_PAGES | FUSE_ASYNC_READ,
    };
    let mut req = fuse::as_bytes(&header).to_vec();
    req.extend_from_slice(fuse::as_bytes(&init_in));

    let resp = proc.handle_request(&req);
    let out: FuseOutHeader = fuse::read_struct(&resp).unwrap();
    assert_eq!(out.error, 0);
    let init_out: FuseInitOut = fuse::read_struct(&resp[16..]).unwrap();
    assert_eq!(init_out.max_readahead, 1024 * 1024);
    assert_eq!(init_out.max_write, 1024 * 1024);
    assert_eq!(init_out.max_pages, 256);
    assert!(init_out.flags & FUSE_BIG_WRITES != 0);
    assert!(init_out.flags & FUSE_MAX_PAGES != 0);
    assert!(init_out.flags & FUSE_ASYNC_READ != 0);
}

// ── Test helpers ─────────────────────────────────────────────────

const HDR_SIZE: usize = std::mem::size_of::<FuseInHeader>();
const OUT_HDR_SIZE: usize = std::mem::size_of::<FuseOutHeader>();
const ENTRY_OUT_SIZE: usize = std::mem::size_of::<FuseEntryOut>();
const ATTR_OUT_SIZE: usize = std::mem::size_of::<FuseAttrOut>();

pub(super) fn make_header(opcode: u32, nodeid: u64, unique: u64) -> FuseInHeader {
    FuseInHeader {
        len: 0,
        opcode,
        unique,
        nodeid,
        uid: 0,
        gid: 0,
        pid: 0,
        padding: 0,
    }
}

pub(super) fn build_request(header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
    let mut req = fuse::as_bytes(header).to_vec();
    req.extend_from_slice(body);
    req
}

pub(super) fn response_error(resp: &[u8]) -> i32 {
    fuse::read_struct::<FuseOutHeader>(resp).unwrap().error
}

/// LOOKUP a name under a parent inode, return the entry's nodeid.
pub(super) fn lookup(proc: &mut FuseProcessor, parent: u64, name: &str) -> Result<u64, i32> {
    let h = make_header(FUSE_LOOKUP, parent, 100);
    let mut body = name.as_bytes().to_vec();
    body.push(0);
    let resp = proc.handle_request(&build_request(&h, &body));
    let out: FuseOutHeader = fuse::read_struct(&resp).unwrap();
    if out.error != 0 {
        return Err(out.error);
    }
    let entry: FuseEntryOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    Ok(entry.nodeid)
}

/// OPEN a file by inode, return the file handle.
pub(super) fn open_file(proc: &mut FuseProcessor, nodeid: u64, flags: u32) -> Result<u64, i32> {
    let h = make_header(FUSE_OPEN, nodeid, 200);
    let open_in = FuseOpenIn { flags, open_flags: 0 };
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&open_in)));
    let out: FuseOutHeader = fuse::read_struct(&resp).unwrap();
    if out.error != 0 {
        return Err(out.error);
    }
    let open_out: FuseOpenOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    Ok(open_out.fh)
}

pub(super) fn open_dir(proc: &mut FuseProcessor, nodeid: u64) -> Result<u64, i32> {
    let h = make_header(FUSE_OPENDIR, nodeid, 201);
    let resp = proc.handle_request(&build_request(&h, &[]));
    let out: FuseOutHeader = fuse::read_struct(&resp).unwrap();
    if out.error != 0 {
        return Err(out.error);
    }
    let open_out: FuseOpenOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    Ok(open_out.fh)
}

#[test]
fn checkpoint_roundtrip_preserves_inode_and_open_handle_state() {
    let dir = temp_share("checkpoint-state-roundtrip");
    std::fs::write(dir.join("held.txt"), b"before\n").unwrap();
    std::fs::write(dir.join("next.txt"), b"next").unwrap();
    let mut proc = test_processor(&dir);
    let held_ino = lookup(&mut proc, 1, "held.txt").unwrap();
    assert_eq!(lookup(&mut proc, 1, "held.txt").unwrap(), held_ino);
    let file_fh = open_file(&mut proc, held_ino, (libc::O_WRONLY | libc::O_APPEND) as u32).unwrap();
    let dir_fh = open_dir(&mut proc, 1).unwrap();
    proc.file_handles
        .get_file(file_fh)
        .unwrap()
        .seek(SeekFrom::Start(3))
        .unwrap();

    let encoded = proc.encode_checkpoint().unwrap();
    let decoded = VirtioFsBackendSnapshot::decode(&encoded).unwrap();
    let expected_dir_entries = match &decoded
        .file_handles
        .handles
        .iter()
        .find(|handle| handle.fh == dir_fh)
        .unwrap()
        .kind
    {
        FileHandleKindSnapshot::Dir { entries, .. } => entries.clone(),
        _ => panic!("expected directory handle"),
    };
    let expected_next_ino = decoded.inodes.next_ino;
    let expected_next_fh = decoded.file_handles.next_fh;
    let mut restored = FuseProcessor::restore_checkpoint(&dir, false, &encoded).unwrap();
    assert_eq!(
        restored.file_handles.get_dir(dir_fh).unwrap(),
        &expected_dir_entries,
        "directory entry snapshot order must be preserved verbatim"
    );
    let seek = FuseLseekIn {
        fh: file_fh,
        offset: 0,
        whence: libc::SEEK_CUR as u32,
        padding: 0,
    };
    let response = restored.do_lseek(&make_header(FUSE_LSEEK, held_ino, 204), fuse::as_bytes(&seek));
    assert_eq!(response_error(&response), 0);
    let seek_out: FuseLseekOut = fuse::read_struct(&response[OUT_HDR_SIZE..]).unwrap();
    assert_eq!(seek_out.offset, 3);

    restored.inodes.forget(held_ino, 1);
    assert!(restored.inodes.get(held_ino).is_some());
    restored.inodes.forget(held_ino, 1);
    assert!(restored.inodes.get(held_ino).is_none());
    assert_eq!(lookup(&mut restored, 1, "next.txt").unwrap(), expected_next_ino);

    let write = FuseWriteIn {
        fh: file_fh,
        offset: 0,
        size: 6,
        write_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let mut body = fuse::as_bytes(&write).to_vec();
    body.extend_from_slice(b"after\n");
    let response = restored.do_write(&make_header(FUSE_WRITE, held_ino, 202), &body);
    assert_eq!(response_error(&response), 0);
    assert_eq!(std::fs::read(dir.join("held.txt")).unwrap(), b"before\nafter\n");

    let read = FuseReadIn {
        fh: dir_fh,
        offset: 0,
        size: 4096,
        read_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let response = restored.do_readdir(&make_header(FUSE_READDIR, 1, 203), fuse::as_bytes(&read));
    assert_eq!(response_error(&response), 0);
    assert!(response.windows(b"held.txt".len()).any(|w| w == b"held.txt"));
    assert_eq!(open_dir(&mut restored, 1).unwrap(), expected_next_fh);
}

#[test]
fn checkpoint_restore_rejects_inode_path_escaping_share_root() {
    let dir = temp_share("checkpoint-state-escape");
    let outside = temp_share("checkpoint-state-outside");
    std::fs::write(outside.join("secret"), b"secret").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("escape")).unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "escape").unwrap();
    let encoded = proc.encode_checkpoint().unwrap();
    let mut decoded = VirtioFsBackendSnapshot::decode(&encoded).unwrap();
    decoded
        .inodes
        .entries
        .iter_mut()
        .find(|entry| entry.ino == ino)
        .unwrap()
        .relative_path = b"escape/secret".to_vec();

    let err = match FuseProcessor::restore_checkpoint(&dir, false, &decoded.encode().unwrap()) {
        Ok(_) => panic!("escaped inode path must be rejected"),
        Err(err) => err,
    };

    assert!(format!("{err:#}").contains("outside VirtioFS root"), "{err:#}");
}

#[test]
fn checkpoint_restore_rejects_replaced_open_file() {
    let dir = temp_share("checkpoint-state-replaced-file");
    let path = dir.join("held.txt");
    std::fs::write(&path, b"before").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "held.txt").unwrap();
    open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();
    let encoded = proc.encode_checkpoint().unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"replacement").unwrap();

    let err = match FuseProcessor::restore_checkpoint(&dir, false, &encoded) {
        Ok(_) => panic!("replaced open file must be rejected"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("identity changed"), "{err:#}");
}

#[test]
fn checkpoint_rejects_open_but_unlinked_file() {
    let dir = temp_share("checkpoint-state-unlinked-file");
    let path = dir.join("held.txt");
    std::fs::write(&path, b"before").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "held.txt").unwrap();
    open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();
    std::fs::remove_file(&path).unwrap();

    let err = proc.encode_checkpoint().unwrap_err();

    assert!(err.to_string().contains("not reopenable"), "{err:#}");
}

#[test]
fn checkpoint_omits_unlinked_cache_only_inode_without_reusing_its_id() {
    let dir = temp_share("checkpoint-state-unlinked-cache-only");
    let stale_path = dir.join("uv-temporary-directory");
    std::fs::create_dir(&stale_path).unwrap();
    let mut proc = test_processor(&dir);
    let stale_ino = lookup(&mut proc, 1, "uv-temporary-directory").unwrap();
    std::fs::remove_dir(&stale_path).unwrap();

    let encoded = proc.encode_checkpoint().unwrap();
    let decoded = VirtioFsBackendSnapshot::decode(&encoded).unwrap();
    assert!(
        decoded.inodes.entries.iter().all(|entry| entry.ino != stale_ino),
        "an unreachable cache-only inode must not block or enter the checkpoint"
    );
    let expected_next_ino = decoded.inodes.next_ino;

    let mut restored = FuseProcessor::restore_checkpoint(&dir, false, &encoded).unwrap();
    assert!(restored.inodes.get(stale_ino).is_none());
    std::fs::create_dir(&stale_path).unwrap();
    assert_eq!(
        lookup(&mut restored, 1, "uv-temporary-directory").unwrap(),
        expected_next_ino,
        "a path recreated after resume must get a fresh inode identity"
    );
}

#[test]
fn checkpoint_restore_rejects_replaced_cached_root_inode() {
    let dir = temp_share("checkpoint-state-replaced-root");
    let moved = dir.with_extension("original");
    let _ = std::fs::remove_dir_all(&moved);
    let mut proc = test_processor(&dir);
    let encoded = proc.encode_checkpoint().unwrap();
    std::fs::rename(&dir, &moved).unwrap();
    std::fs::create_dir(&dir).unwrap();

    let err = match FuseProcessor::restore_checkpoint(&dir, false, &encoded) {
        Ok(_) => panic!("replaced root inode must be rejected"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("identity changed"), "{err:#}");
}

#[test]
fn checkpoint_restore_rejects_replaced_directory_handle() {
    let dir = temp_share("checkpoint-state-replaced-directory");
    std::fs::create_dir(dir.join("held-dir")).unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "held-dir").unwrap();
    open_dir(&mut proc, ino).unwrap();
    let encoded = proc.encode_checkpoint().unwrap();
    std::fs::rename(dir.join("held-dir"), dir.join("old-dir")).unwrap();
    std::fs::create_dir(dir.join("held-dir")).unwrap();

    let err = match FuseProcessor::restore_checkpoint(&dir, false, &encoded) {
        Ok(_) => panic!("replaced directory handle must be rejected"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("identity changed"), "{err:#}");
}

#[test]
fn checkpoint_restore_rejects_fifo_before_reopen() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let dir = temp_share("checkpoint-state-fifo");
    let path = dir.join("held.txt");
    std::fs::write(&path, b"before").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "held.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();
    let encoded = proc.encode_checkpoint().unwrap();
    let mut decoded = VirtioFsBackendSnapshot::decode(&encoded).unwrap();
    std::fs::remove_file(&path).unwrap();
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    let metadata = std::fs::symlink_metadata(&path).unwrap();
    let inode = decoded
        .inodes
        .entries
        .iter_mut()
        .find(|entry| entry.ino == ino)
        .unwrap();
    inode.device = metadata.dev();
    inode.host_inode = metadata.ino();
    inode.file_type = host_file_type(&metadata);
    let handle = decoded
        .file_handles
        .handles
        .iter_mut()
        .find(|handle| handle.fh == fh)
        .unwrap();
    if let FileHandleKindSnapshot::File {
        device,
        inode,
        file_type,
        ..
    } = &mut handle.kind
    {
        *device = metadata.dev();
        *inode = metadata.ino();
        *file_type = host_file_type(&metadata);
    }

    let err = match FuseProcessor::restore_checkpoint(&dir, false, &decoded.encode().unwrap()) {
        Ok(_) => panic!("FIFO checkpoint must be rejected"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("not a regular file"), "{err:#}");
}

#[test]
fn virtiofs_device_restore_rejects_tag_and_read_only_identity_mismatch() {
    let dir = temp_share("checkpoint-device-identity");
    let mut source = test_device(&dir);
    source.quiesce().unwrap();
    let state = source.checkpoint_state().unwrap();

    let mut wrong_tag = VirtioFsDevice::new("other", &dir, false, -1, Arc::new(AtomicU32::new(0))).unwrap();
    let tag_err = wrong_tag.restore_checkpoint_state(&state).unwrap_err();
    assert!(tag_err.to_string().contains("tag identity mismatch"), "{tag_err:#}");

    let mut wrong_mode = VirtioFsDevice::new("capsem", &dir, true, -1, Arc::new(AtomicU32::new(0))).unwrap();
    let mode_err = wrong_mode.restore_checkpoint_state(&state).unwrap_err();
    assert!(
        mode_err.to_string().contains("read-only identity mismatch"),
        "{mode_err:#}"
    );
}

#[test]
fn checkpoint_codec_rejects_invalid_boolean_and_count_before_allocation() {
    let dir = temp_share("checkpoint-codec-bounds");
    let mut proc = test_processor(&dir);
    let encoded = proc.encode_checkpoint().unwrap();

    let mut invalid_boolean = encoded.clone();
    invalid_boolean[8 + 4 + TAG_LEN] = 2;
    let err = VirtioFsBackendSnapshot::decode(&invalid_boolean).unwrap_err();
    assert!(err.to_string().contains("boolean"), "{err:#}");

    let mut invalid_count = encoded;
    let count_offset = 8 + 4 + TAG_LEN + 1;
    invalid_count[count_offset..count_offset + 4].copy_from_slice(&1_048_577u32.to_le_bytes());
    let err = VirtioFsBackendSnapshot::decode(&invalid_count).unwrap_err();
    assert!(err.to_string().contains("inode count exceeds limit"), "{err:#}");
}

#[test]
fn checkpoint_restore_rejects_symlink_replacement_without_following_it() {
    let dir = temp_share("checkpoint-state-symlink-replacement");
    let outside = temp_share("checkpoint-state-symlink-target");
    let path = dir.join("held.txt");
    let outside_path = outside.join("outside.txt");
    std::fs::write(&path, b"inside").unwrap();
    std::fs::write(&outside_path, b"outside").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "held.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();
    let encoded = proc.encode_checkpoint().unwrap();
    let mut decoded = VirtioFsBackendSnapshot::decode(&encoded).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside_path, &path).unwrap();
    let metadata = std::fs::symlink_metadata(&path).unwrap();
    let inode = decoded
        .inodes
        .entries
        .iter_mut()
        .find(|entry| entry.ino == ino)
        .unwrap();
    inode.device = metadata.dev();
    inode.host_inode = metadata.ino();
    inode.file_type = host_file_type(&metadata);
    assert!(decoded.file_handles.handles.iter().any(|handle| handle.fh == fh));

    let err = match FuseProcessor::restore_checkpoint(&dir, false, &decoded.encode().unwrap()) {
        Ok(_) => panic!("symlink replacement must be rejected"),
        Err(err) => err,
    };

    assert!(format!("{err:#}").contains("outside VirtioFS root"), "{err:#}");
}

// ── ops_meta tests ───────────────────────────────────────────────

#[test]
fn lookup_existing_file() {
    let dir = temp_share("lookup-exist");
    std::fs::write(dir.join("hello.txt"), b"data").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "hello.txt").unwrap();
    assert!(ino > 1, "lookup should return a valid inode");
}

#[test]
fn lookup_nonexistent() {
    let dir = temp_share("lookup-none");
    let mut proc = test_processor(&dir);
    let err = lookup(&mut proc, 1, "nope.txt").unwrap_err();
    assert_eq!(err, -libc::ENOENT);
}

#[test]
fn getattr_root() {
    let dir = temp_share("getattr-root");
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_GETATTR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    let attr_out: FuseAttrOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert_ne!(attr_out.attr.mode & S_IFDIR, 0, "root should be a directory");
}

#[test]
fn getattr_file() {
    let dir = temp_share("getattr-file");
    std::fs::write(dir.join("f.txt"), b"12345").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();

    let h = make_header(FUSE_GETATTR, ino, 2);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    let attr_out: FuseAttrOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert_eq!(attr_out.attr.size, 5);
    assert_ne!(attr_out.attr.mode & S_IFREG, 0);
}

#[test]
fn getattr_nonexistent_inode() {
    let dir = temp_share("getattr-bad");
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_GETATTR, 99999, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), -libc::ENOENT);
}

#[test]
fn setattr_chmod() {
    let dir = temp_share("setattr-chmod");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();

    let attr_in = FuseSetAttrIn {
        valid: FATTR_MODE,
        padding: 0,
        fh: 0,
        size: 0,
        lock_owner: 0,
        atime: 0,
        mtime: 0,
        ctime: 0,
        atimensec: 0,
        mtimensec: 0,
        ctimensec: 0,
        mode: 0o755,
        unused4: 0,
        uid: 0,
        gid: 0,
        unused5: 0,
    };
    let h = make_header(FUSE_SETATTR, ino, 3);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&attr_in)));
    assert_eq!(response_error(&resp), 0);

    let perms = std::fs::metadata(dir.join("f.txt")).unwrap().permissions();
    assert_eq!(perms.mode() & 0o777, 0o755);
}

#[test]
fn setattr_truncate() {
    let dir = temp_share("setattr-trunc");
    std::fs::write(dir.join("big.txt"), b"hello world").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "big.txt").unwrap();

    let attr_in = FuseSetAttrIn {
        valid: FATTR_SIZE,
        padding: 0,
        fh: 0,
        size: 3,
        lock_owner: 0,
        atime: 0,
        mtime: 0,
        ctime: 0,
        atimensec: 0,
        mtimensec: 0,
        ctimensec: 0,
        mode: 0,
        unused4: 0,
        uid: 0,
        gid: 0,
        unused5: 0,
    };
    let h = make_header(FUSE_SETATTR, ino, 4);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&attr_in)));
    assert_eq!(response_error(&resp), 0);

    let content = std::fs::read(dir.join("big.txt")).unwrap();
    assert_eq!(content.len(), 3);
    assert_eq!(&content, b"hel");
}

#[test]
fn setattr_read_only_rejected() {
    let dir = temp_share("setattr-ro");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    proc.read_only = true;
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();

    let attr_in = FuseSetAttrIn {
        valid: FATTR_MODE,
        padding: 0,
        fh: 0,
        size: 0,
        lock_owner: 0,
        atime: 0,
        mtime: 0,
        ctime: 0,
        atimensec: 0,
        mtimensec: 0,
        ctimensec: 0,
        mode: 0o777,
        unused4: 0,
        uid: 0,
        gid: 0,
        unused5: 0,
    };
    let h = make_header(FUSE_SETATTR, ino, 5);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&attr_in)));
    assert_eq!(response_error(&resp), -libc::EROFS);
}

#[test]
fn statfs_returns_data() {
    let dir = temp_share("statfs");
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_STATFS, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    let kstatfs: FuseKStatfs = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert!(kstatfs.blocks > 0, "statfs should report non-zero blocks");
    assert!(kstatfs.bsize > 0, "statfs should report non-zero block size");
}

#[test]
fn forget_does_not_crash() {
    let dir = temp_share("forget");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();

    // FORGET for a valid inode
    let h = make_header(FUSE_FORGET, ino, 1);
    let forget_in = FuseForgetIn { nlookup: 1 };
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&forget_in)));
    assert!(resp.is_empty(), "FORGET should produce no response");

    // FORGET for a nonexistent inode -- should not panic
    let h2 = make_header(FUSE_FORGET, 99999, 2);
    let resp2 = proc.handle_request(&build_request(&h2, fuse::as_bytes(&forget_in)));
    assert!(resp2.is_empty());
}

#[test]
fn batch_forget_multiple() {
    let dir = temp_share("batch-forget");
    std::fs::write(dir.join("a.txt"), b"a").unwrap();
    std::fs::write(dir.join("b.txt"), b"b").unwrap();
    let mut proc = test_processor(&dir);
    let ino_a = lookup(&mut proc, 1, "a.txt").unwrap();
    let ino_b = lookup(&mut proc, 1, "b.txt").unwrap();

    let h = make_header(FUSE_BATCH_FORGET, 0, 1);
    let batch = FuseBatchForgetIn { count: 2, dummy: 0 };
    let e1 = FuseForgetOne {
        nodeid: ino_a,
        nlookup: 1,
    };
    let e2 = FuseForgetOne {
        nodeid: ino_b,
        nlookup: 1,
    };
    let mut body = fuse::as_bytes(&batch).to_vec();
    body.extend_from_slice(fuse::as_bytes(&e1));
    body.extend_from_slice(fuse::as_bytes(&e2));

    let resp = proc.handle_request(&build_request(&h, &body));
    assert!(resp.is_empty(), "BATCH_FORGET should produce no response");
}

// ── ops_file tests ───────────────────────────────────────────────

#[test]
fn open_read_write_release() {
    let dir = temp_share("file-lifecycle");
    std::fs::write(dir.join("rw.txt"), b"initial").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "rw.txt").unwrap();

    // OPEN for read+write
    let fh = open_file(&mut proc, ino, libc::O_RDWR as u32).unwrap();

    // READ
    let read_in = FuseReadIn {
        fh,
        offset: 0,
        size: 1024,
        read_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_READ, ino, 10);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(&resp[OUT_HDR_SIZE..], b"initial");

    // WRITE at offset 0
    let write_in = FuseWriteIn {
        fh,
        offset: 0,
        size: 7,
        write_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_WRITE, ino, 11);
    let mut body = fuse::as_bytes(&write_in).to_vec();
    body.extend_from_slice(b"updated");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);
    let write_out: FuseWriteOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert_eq!(write_out.size, 7);

    // RELEASE
    let release_in = FuseReleaseIn {
        fh,
        flags: 0,
        release_flags: 0,
        lock_owner: 0,
    };
    let h = make_header(FUSE_RELEASE, ino, 12);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&release_in)));
    assert_eq!(response_error(&resp), 0);

    // Verify on disk
    assert_eq!(std::fs::read(dir.join("rw.txt")).unwrap(), b"updated");
}

#[test]
fn open_nonexistent() {
    let dir = temp_share("open-none");
    let mut proc = test_processor(&dir);
    let err = open_file(&mut proc, 99999, libc::O_RDONLY as u32).unwrap_err();
    assert_eq!(err, -libc::ENOENT);
}

#[test]
fn open_write_on_readonly() {
    let dir = temp_share("open-ro");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    proc.read_only = true;
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    let err = open_file(&mut proc, ino, libc::O_WRONLY as u32).unwrap_err();
    assert_eq!(err, -libc::EROFS);
}

#[test]
fn read_with_offset() {
    let dir = temp_share("read-offset");
    std::fs::write(dir.join("data.txt"), b"abcdefghij").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "data.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();

    let read_in = FuseReadIn {
        fh,
        offset: 5,
        size: 100,
        read_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_READ, ino, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(&resp[OUT_HDR_SIZE..], b"fghij");
}

#[test]
fn read_past_eof_returns_empty() {
    let dir = temp_share("read-eof");
    std::fs::write(dir.join("small.txt"), b"hi").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "small.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();

    let read_in = FuseReadIn {
        fh,
        offset: 100,
        size: 100,
        read_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_READ, ino, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(resp.len(), OUT_HDR_SIZE, "read past EOF should return empty body");
}

#[test]
fn read_write_use_positional_io_without_moving_handle_cursor() {
    let dir = temp_share("positional-io");
    std::fs::write(dir.join("data.txt"), b"abcdefghij").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "data.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDWR as u32).unwrap();

    proc.file_handles
        .get_file(fh)
        .unwrap()
        .seek(SeekFrom::Start(7))
        .unwrap();

    let read_in = FuseReadIn {
        fh,
        offset: 0,
        size: 3,
        read_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_READ, ino, 20);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(&resp[OUT_HDR_SIZE..], b"abc");
    assert_eq!(proc.file_handles.get_file(fh).unwrap().stream_position().unwrap(), 7);

    let write_in = FuseWriteIn {
        fh,
        offset: 1,
        size: 3,
        write_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_WRITE, ino, 21);
    let mut body = fuse::as_bytes(&write_in).to_vec();
    body.extend_from_slice(b"XYZ");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(proc.file_handles.get_file(fh).unwrap().stream_position().unwrap(), 7);
    assert_eq!(std::fs::read(dir.join("data.txt")).unwrap(), b"aXYZefghij");
}

#[test]
fn write_on_readonly_rejected() {
    let dir = temp_share("write-ro");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    proc.read_only = true;

    let write_in = FuseWriteIn {
        fh: 0,
        offset: 0,
        size: 3,
        write_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_WRITE, ino, 1);
    let mut body = fuse::as_bytes(&write_in).to_vec();
    body.extend_from_slice(b"bad");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), -libc::EROFS);
}

#[test]
fn create_new_file() {
    let dir = temp_share("create-new");
    let mut proc = test_processor(&dir);

    let create_in = FuseCreateIn {
        flags: libc::O_RDWR as u32,
        mode: 0o644,
        umask: 0,
        open_flags: 0,
    };
    let h = make_header(FUSE_CREATE, 1, 1);
    let mut body = fuse::as_bytes(&create_in).to_vec();
    body.extend_from_slice(b"newfile.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);

    // File should exist on disk
    assert!(dir.join("newfile.txt").exists());

    // Response should contain entry + open
    let entry: FuseEntryOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert!(entry.nodeid > 0);
    let open_out: FuseOpenOut = fuse::read_struct(&resp[OUT_HDR_SIZE + ENTRY_OUT_SIZE..]).unwrap();
    assert!(open_out.fh > 0);
}

#[test]
fn create_new_append_handle_preserves_semantics_across_checkpoint() {
    let dir = temp_share("create-new-append-checkpoint");
    let mut proc = test_processor(&dir);

    let create_in = FuseCreateIn {
        flags: (libc::O_WRONLY | libc::O_APPEND) as u32,
        mode: 0o644,
        umask: 0,
        open_flags: 0,
    };
    let header = make_header(FUSE_CREATE, 1, 1);
    let mut body = fuse::as_bytes(&create_in).to_vec();
    body.extend_from_slice(b"append.txt\0");
    let response = proc.handle_request(&build_request(&header, &body));
    assert_eq!(response_error(&response), 0);
    let entry: FuseEntryOut = fuse::read_struct(&response[OUT_HDR_SIZE..]).unwrap();
    let opened: FuseOpenOut = fuse::read_struct(&response[OUT_HDR_SIZE + ENTRY_OUT_SIZE..]).unwrap();

    std::fs::write(dir.join("append.txt"), b"base\n").unwrap();
    let write = |processor: &mut FuseProcessor, unique, data: &[u8]| {
        let input = FuseWriteIn {
            fh: opened.fh,
            offset: 0,
            size: data.len() as u32,
            write_flags: 0,
            lock_owner: 0,
            flags: 0,
            padding: 0,
        };
        let mut request = fuse::as_bytes(&input).to_vec();
        request.extend_from_slice(data);
        let response =
            processor.handle_request(&build_request(&make_header(FUSE_WRITE, entry.nodeid, unique), &request));
        assert_eq!(response_error(&response), 0);
    };

    write(&mut proc, 2, b"before\n");
    let checkpoint = proc.encode_checkpoint().unwrap();
    let mut restored = FuseProcessor::restore_checkpoint(&dir, false, &checkpoint).unwrap();
    write(&mut restored, 3, b"after\n");

    assert_eq!(std::fs::read(dir.join("append.txt")).unwrap(), b"base\nbefore\nafter\n");
}

#[test]
fn create_readonly_rejected() {
    let dir = temp_share("create-ro");
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let create_in = FuseCreateIn {
        flags: libc::O_RDWR as u32,
        mode: 0o644,
        umask: 0,
        open_flags: 0,
    };
    let h = make_header(FUSE_CREATE, 1, 1);
    let mut body = fuse::as_bytes(&create_in).to_vec();
    body.extend_from_slice(b"nope.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), -libc::EROFS);
    assert!(!dir.join("nope.txt").exists());
}

#[test]
fn create_existing_file_opens_it() {
    let dir = temp_share("create-exist");
    std::fs::write(dir.join("exist.txt"), b"old content").unwrap();
    let mut proc = test_processor(&dir);

    let create_in = FuseCreateIn {
        flags: libc::O_RDWR as u32,
        mode: 0o644,
        umask: 0,
        open_flags: 0,
    };
    let h = make_header(FUSE_CREATE, 1, 1);
    let mut body = fuse::as_bytes(&create_in).to_vec();
    body.extend_from_slice(b"exist.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);
}

#[test]
fn flush_and_fsync() {
    let dir = temp_share("flush-fsync");
    std::fs::write(dir.join("f.txt"), b"data").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDWR as u32).unwrap();

    // FLUSH
    let flush_in = FuseFlushIn {
        fh,
        unused: 0,
        padding: 0,
        lock_owner: 0,
    };
    let h = make_header(FUSE_FLUSH, ino, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&flush_in)));
    assert_eq!(response_error(&resp), 0);

    // FSYNC (data-only)
    let fsync_in = FuseFsyncIn {
        fh,
        fsync_flags: 1,
        padding: 0,
    };
    let h = make_header(FUSE_FSYNC, ino, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&fsync_in)));
    assert_eq!(response_error(&resp), 0);

    // FSYNC (full)
    let fsync_in = FuseFsyncIn {
        fh,
        fsync_flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_FSYNC, ino, 3);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&fsync_in)));
    assert_eq!(response_error(&resp), 0);
}

#[test]
fn flush_bad_handle() {
    let dir = temp_share("flush-bad");
    let mut proc = test_processor(&dir);
    let flush_in = FuseFlushIn {
        fh: 99999,
        unused: 0,
        padding: 0,
        lock_owner: 0,
    };
    let h = make_header(FUSE_FLUSH, 1, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&flush_in)));
    assert_eq!(response_error(&resp), -libc::EBADF);
}

#[test]
fn lseek_whence() {
    let dir = temp_share("lseek");
    std::fs::write(dir.join("seek.txt"), b"0123456789").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "seek.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();

    // SEEK_SET to offset 5
    let lseek_in = FuseLseekIn {
        fh,
        offset: 5,
        whence: 0,
        padding: 0,
    };
    let h = make_header(FUSE_LSEEK, ino, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&lseek_in)));
    assert_eq!(response_error(&resp), 0);
    let out: FuseLseekOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert_eq!(out.offset, 5);

    // SEEK_END to offset 0 (should be at position 10)
    let lseek_in = FuseLseekIn {
        fh,
        offset: 0,
        whence: 2,
        padding: 0,
    };
    let h = make_header(FUSE_LSEEK, ino, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&lseek_in)));
    assert_eq!(response_error(&resp), 0);
    let out: FuseLseekOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert_eq!(out.offset, 10);
}

#[test]
fn lseek_invalid_whence() {
    let dir = temp_share("lseek-bad");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();

    let lseek_in = FuseLseekIn {
        fh,
        offset: 0,
        whence: 99,
        padding: 0,
    };
    let h = make_header(FUSE_LSEEK, ino, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&lseek_in)));
    assert_eq!(response_error(&resp), -libc::EINVAL);
}

#[test]
fn read_bad_handle() {
    let dir = temp_share("read-bad-fh");
    let mut proc = test_processor(&dir);
    let read_in = FuseReadIn {
        fh: 99999,
        offset: 0,
        size: 100,
        read_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_READ, 1, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), -libc::EBADF);
}

// ── ops_dir tests ────────────────────────────────────────────────

#[test]
fn opendir_readdir_releasedir() {
    let dir = temp_share("dir-lifecycle");
    std::fs::write(dir.join("a.txt"), b"a").unwrap();
    std::fs::write(dir.join("b.txt"), b"b").unwrap();
    let mut proc = test_processor(&dir);

    // OPENDIR on root
    let h = make_header(FUSE_OPENDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    let open_out: FuseOpenOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    let fh = open_out.fh;

    // READDIR
    let read_in = FuseReadIn {
        fh,
        offset: 0,
        size: 4096,
        read_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_READDIR, 1, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    // Should have data (. + .. + a.txt + b.txt = 4 entries)
    assert!(resp.len() > OUT_HDR_SIZE);

    // RELEASEDIR
    let release_in = FuseReleaseIn {
        fh,
        flags: 0,
        release_flags: 0,
        lock_owner: 0,
    };
    let h = make_header(FUSE_RELEASEDIR, 1, 3);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&release_in)));
    assert_eq!(response_error(&resp), 0);
}

#[test]
fn readdir_includes_dot_dotdot() {
    let dir = temp_share("readdir-dots");
    let mut proc = test_processor(&dir);

    // OPENDIR
    let h = make_header(FUSE_OPENDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    let open_out: FuseOpenOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    let fh = open_out.fh;

    // READDIR
    let read_in = FuseReadIn {
        fh,
        offset: 0,
        size: 4096,
        read_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_READDIR, 1, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    let body = &resp[OUT_HDR_SIZE..];

    // Parse first two dirents -- should be "." and ".."
    let dirent_size = std::mem::size_of::<FuseDirent>();
    let d1: FuseDirent = fuse::read_struct(body).unwrap();
    let name1 = &body[dirent_size..dirent_size + d1.namelen as usize];
    assert_eq!(name1, b".");

    let entry1_size = fuse::dirent_align(dirent_size + d1.namelen as usize);
    let d2: FuseDirent = fuse::read_struct(&body[entry1_size..]).unwrap();
    let name2 = &body[entry1_size + dirent_size..entry1_size + dirent_size + d2.namelen as usize];
    assert_eq!(name2, b"..");
}

#[test]
fn opendir_nonexistent() {
    let dir = temp_share("opendir-bad");
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_OPENDIR, 99999, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), -libc::ENOENT);
}

#[test]
fn mkdir_creates_directory() {
    let dir = temp_share("mkdir");
    let mut proc = test_processor(&dir);

    let mkdir_in = FuseMkdirIn { mode: 0o755, umask: 0 };
    let h = make_header(FUSE_MKDIR, 1, 1);
    let mut body = fuse::as_bytes(&mkdir_in).to_vec();
    body.extend_from_slice(b"subdir\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);

    assert!(dir.join("subdir").is_dir());
    let entry: FuseEntryOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert!(entry.nodeid > 0);
}

#[test]
fn mkdir_readonly_rejected() {
    let dir = temp_share("mkdir-ro");
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let mkdir_in = FuseMkdirIn { mode: 0o755, umask: 0 };
    let h = make_header(FUSE_MKDIR, 1, 1);
    let mut body = fuse::as_bytes(&mkdir_in).to_vec();
    body.extend_from_slice(b"nope\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), -libc::EROFS);
    assert!(!dir.join("nope").exists());
}

#[test]
fn unlink_removes_file() {
    let dir = temp_share("unlink");
    std::fs::write(dir.join("doomed.txt"), b"bye").unwrap();
    let mut proc = test_processor(&dir);

    let h = make_header(FUSE_UNLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"doomed.txt\0"));
    assert_eq!(response_error(&resp), 0);
    assert!(!dir.join("doomed.txt").exists());
}

#[test]
fn unlink_nonexistent() {
    let dir = temp_share("unlink-none");
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_UNLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"nope.txt\0"));
    assert_ne!(response_error(&resp), 0);
}

#[test]
fn unlink_readonly_rejected() {
    let dir = temp_share("unlink-ro");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let h = make_header(FUSE_UNLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"f.txt\0"));
    assert_eq!(response_error(&resp), -libc::EROFS);
    assert!(dir.join("f.txt").exists());
}

#[test]
fn rmdir_removes_directory() {
    let dir = temp_share("rmdir");
    std::fs::create_dir(dir.join("empty_dir")).unwrap();
    let mut proc = test_processor(&dir);

    let h = make_header(FUSE_RMDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"empty_dir\0"));
    assert_eq!(response_error(&resp), 0);
    assert!(!dir.join("empty_dir").exists());
}

#[test]
fn rmdir_readonly_rejected() {
    let dir = temp_share("rmdir-ro");
    std::fs::create_dir(dir.join("d")).unwrap();
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let h = make_header(FUSE_RMDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"d\0"));
    assert_eq!(response_error(&resp), -libc::EROFS);
    assert!(dir.join("d").exists());
}

#[test]
fn rename_file() {
    let dir = temp_share("rename");
    std::fs::write(dir.join("old.txt"), b"content").unwrap();
    let mut proc = test_processor(&dir);

    // RENAME: old.txt -> new.txt (both in root, nodeid=1)
    let rename_in = FuseRenameIn { newdir: 1 };
    let h = make_header(FUSE_RENAME, 1, 1);
    let mut body = fuse::as_bytes(&rename_in).to_vec();
    body.extend_from_slice(b"old.txt\0new.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);

    assert!(!dir.join("old.txt").exists());
    assert_eq!(std::fs::read(dir.join("new.txt")).unwrap(), b"content");
}

#[test]
fn rename_over_existing_rebinds_source_inode_to_target_path() {
    let dir = temp_share("rename-over-existing");
    std::fs::write(dir.join("config.json"), b"old").unwrap();
    std::fs::write(dir.join("config.json.tmp"), b"new").unwrap();
    let mut proc = test_processor(&dir);
    let _target_ino = lookup(&mut proc, 1, "config.json").unwrap();
    let temp_ino = lookup(&mut proc, 1, "config.json.tmp").unwrap();

    let rename_in = FuseRenameIn { newdir: 1 };
    let h = make_header(FUSE_RENAME, 1, 1);
    let mut body = fuse::as_bytes(&rename_in).to_vec();
    body.extend_from_slice(b"config.json.tmp\0config.json\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);

    let fh = open_file(&mut proc, temp_ino, libc::O_RDONLY as u32).unwrap();
    let read_in = FuseReadIn {
        fh,
        offset: 0,
        size: 1024,
        read_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_READ, temp_ino, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(&resp[OUT_HDR_SIZE..], b"new");
}

#[test]
fn rename_readonly_rejected() {
    let dir = temp_share("rename-ro");
    std::fs::write(dir.join("a.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let rename_in = FuseRenameIn { newdir: 1 };
    let h = make_header(FUSE_RENAME, 1, 1);
    let mut body = fuse::as_bytes(&rename_in).to_vec();
    body.extend_from_slice(b"a.txt\0b.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), -libc::EROFS);
    assert!(dir.join("a.txt").exists());
}

#[test]
fn symlink_and_readlink() {
    let dir = temp_share("symlink");
    std::fs::write(dir.join("target.txt"), b"real").unwrap();
    let mut proc = test_processor(&dir);

    // SYMLINK: create "link.txt" -> "target.txt".
    let h = make_header(FUSE_SYMLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"link.txt\0target.txt\0"));
    assert_eq!(response_error(&resp), 0);
    let entry: FuseEntryOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    let link_ino = entry.nodeid;

    // READLINK
    let h = make_header(FUSE_READLINK, link_ino, 2);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(&resp[OUT_HDR_SIZE..], b"target.txt");
}

#[test]
fn linux_readlink_opcode_is_five_not_getxattr() {
    let dir = temp_share("symlink-opcode");
    std::fs::write(dir.join("target.txt"), b"real").unwrap();
    let mut proc = test_processor(&dir);

    let h = make_header(FUSE_SYMLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"link.txt\0target.txt\0"));
    assert_eq!(response_error(&resp), 0);
    let entry: FuseEntryOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();

    let h = make_header(5, entry.nodeid, 2);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(&resp[OUT_HDR_SIZE..], b"target.txt");

    let h = make_header(22, entry.nodeid, 3);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), -libc::ENOSYS);
}

#[test]
fn symlink_readonly_rejected() {
    let dir = temp_share("symlink-ro");
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let h = make_header(FUSE_SYMLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"link\0target\0"));
    assert_eq!(response_error(&resp), -libc::EROFS);
}

#[test]
fn symlink_absolute_target_is_preserved() {
    let dir = temp_share("symlink-escape");
    let mut proc = test_processor(&dir);

    let h = make_header(FUSE_SYMLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"escape\0/etc/passwd\0"));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(
        std::fs::read_link(dir.join("escape")).unwrap(),
        PathBuf::from("/etc/passwd")
    );
}

#[test]
fn link_creates_hardlink() {
    let dir = temp_share("hardlink");
    std::fs::write(dir.join("original.txt"), b"shared").unwrap();
    let mut proc = test_processor(&dir);
    let orig_ino = lookup(&mut proc, 1, "original.txt").unwrap();

    // LINK: create "linked.txt" pointing to original.txt's inode
    let link_in = FuseLinkIn { oldnodeid: orig_ino };
    let h = make_header(FUSE_LINK, 1, 1);
    let mut body = fuse::as_bytes(&link_in).to_vec();
    body.extend_from_slice(b"linked.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);

    // Both files should exist with same content
    assert_eq!(std::fs::read(dir.join("original.txt")).unwrap(), b"shared");
    assert_eq!(std::fs::read(dir.join("linked.txt")).unwrap(), b"shared");
}

#[test]
fn link_readonly_rejected() {
    let dir = temp_share("link-ro");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    proc.read_only = true;

    let link_in = FuseLinkIn { oldnodeid: ino };
    let h = make_header(FUSE_LINK, 1, 1);
    let mut body = fuse::as_bytes(&link_in).to_vec();
    body.extend_from_slice(b"linked.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), -libc::EROFS);
}

#[test]
fn fsyncdir_success() {
    let dir = temp_share("fsyncdir");
    let mut proc = test_processor(&dir);

    // OPENDIR first to get a valid dir handle
    let h = make_header(FUSE_OPENDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    let open_out: FuseOpenOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    let fh = open_out.fh;

    // FSYNCDIR
    let fsync_in = FuseFsyncIn {
        fh,
        fsync_flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_FSYNCDIR, 1, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&fsync_in)));
    assert_eq!(response_error(&resp), 0);
}

#[test]
fn fsyncdir_bad_handle() {
    let dir = temp_share("fsyncdir-bad");
    let mut proc = test_processor(&dir);

    let fsync_in = FuseFsyncIn {
        fh: 99999,
        fsync_flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_FSYNCDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&fsync_in)));
    assert_eq!(response_error(&resp), -libc::EBADF);
}

// ── adversarial tests ────────────────────────────────────────────

#[test]
fn create_path_traversal_rejected() {
    let dir = temp_share("path-traversal");
    let mut proc = test_processor(&dir);

    // Try to create a file with "../" in the name
    let create_in = FuseCreateIn {
        flags: libc::O_RDWR as u32,
        mode: 0o644,
        umask: 0,
        open_flags: 0,
    };
    let h = make_header(FUSE_CREATE, 1, 1);
    let mut body = fuse::as_bytes(&create_in).to_vec();
    body.extend_from_slice(b"../escape.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    // The inode table should reject path traversal
    let err = response_error(&resp);
    assert_ne!(err, 0, "path traversal should be rejected");

    // Verify no file was created outside the share
    let parent = dir.parent().unwrap();
    assert!(
        !parent.join("escape.txt").exists(),
        "file must not escape the shared directory"
    );
}

#[test]
fn unsupported_opcode_returns_enosys() {
    let dir = temp_share("enosys");
    let mut proc = test_processor(&dir);
    let h = make_header(255, 1, 1); // bogus opcode
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), -libc::ENOSYS);
}

#[test]
fn truncated_request_returns_error() {
    let dir = temp_share("truncated");
    let mut proc = test_processor(&dir);
    // Send a valid header for OPEN but with a truncated body
    let h = make_header(FUSE_OPEN, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[0])); // body too short for FuseOpenIn
    assert_ne!(response_error(&resp), 0);
}
