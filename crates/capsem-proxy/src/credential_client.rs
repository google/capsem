use std::io;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::Duration;

use capsem_core::credential_broker::{
    BrokeredCredential, BrokeredUpstreamCredentials, CredentialObservation, CredentialProvider,
};
use capsem_core::net::ai_traffic::provider::ProviderKind;
use capsem_core::net::proxy_engine::ProxyCredentials;
use capsem_foundation::ipc_channel;
use capsem_proto::proxy_control::ProxyChannelCloseReason;
use capsem_proto::proxy_credentials::{
    ProxyBrokeredCredential, ProxyCredentialObservation, ProxyCredentialProvider, ProxyCredentialRequest,
    ProxyCredentialResponse, ProxyHeader, ProxyModelProvider,
};

const COMMAND_CAPACITY: usize = 32;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

type CaptureReply = mpsc::SyncSender<Result<BrokeredCredential, String>>;
type SubstituteReply = mpsc::SyncSender<Result<(http::HeaderMap, BrokeredUpstreamCredentials), String>>;

enum Command {
    Capture {
        observation: CredentialObservation,
        reply: CaptureReply,
    },
    Substitute {
        domain: String,
        ai_provider: Option<ProviderKind>,
        headers: http::HeaderMap,
        query: Option<String>,
        reply: SubstituteReply,
    },
}

impl Command {
    fn fail(self, error: String) {
        match self {
            Self::Capture { reply, .. } => {
                let _ = reply.send(Err(error));
            }
            Self::Substitute { reply, .. } => {
                let _ = reply.send(Err(error));
            }
        }
    }
}

pub(super) struct CredentialClient {
    commands: tokio::sync::mpsc::Sender<Command>,
}

impl CredentialClient {
    pub(super) fn start(
        stream: UnixStream,
    ) -> io::Result<(Self, tokio::sync::watch::Receiver<Option<ProxyChannelCloseReason>>)> {
        let (commands, inbox) = tokio::sync::mpsc::channel(COMMAND_CAPACITY);
        let (closed_tx, closed) = tokio::sync::watch::channel(None);
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("capsem-proxy-credentials".to_string())
            .spawn(move || run(stream, inbox, startup_tx, closed_tx))?;
        match startup_rx.recv_timeout(OPERATION_TIMEOUT) {
            Ok(Ok(())) => Ok((Self { commands }, closed)),
            Ok(Err(error)) => Err(io::Error::other(error)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "proxy credential client startup timed out",
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "proxy credential client stopped during startup",
            )),
        }
    }

    fn send<T>(&self, command: Command, response: mpsc::Receiver<Result<T, String>>) -> Result<T, String> {
        self.commands.try_send(command).map_err(|error| match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => "proxy credential capability is at capacity".to_string(),
            tokio::sync::mpsc::error::TrySendError::Closed(_) => "proxy credential capability is closed".to_string(),
        })?;
        let receive = || {
            response
                .recv_timeout(OPERATION_TIMEOUT)
                .map_err(|_| "proxy credential operation timed out".to_string())?
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(receive)
        } else {
            receive()
        }
    }
}

impl ProxyCredentials for CredentialClient {
    fn capture(&self, observation: &CredentialObservation) -> Result<BrokeredCredential, String> {
        let (reply, response) = mpsc::sync_channel(1);
        self.send(
            Command::Capture {
                observation: observation.clone(),
                reply,
            },
            response,
        )
    }

    fn substitute_upstream(
        &self,
        domain: &str,
        ai_provider: Option<ProviderKind>,
        headers: &mut http::HeaderMap,
        query: Option<&str>,
    ) -> Result<BrokeredUpstreamCredentials, String> {
        let (reply, response) = mpsc::sync_channel(1);
        let (substituted_headers, result) = self.send(
            Command::Substitute {
                domain: domain.to_string(),
                ai_provider,
                headers: headers.clone(),
                query: query.map(str::to_string),
                reply,
            },
            response,
        )?;
        *headers = substituted_headers;
        Ok(result)
    }
}

fn run(
    stream: UnixStream,
    inbox: tokio::sync::mpsc::Receiver<Command>,
    startup: mpsc::SyncSender<Result<(), String>>,
    closed: tokio::sync::watch::Sender<Option<ProxyChannelCloseReason>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = startup.send(Err(format!("build proxy credential runtime: {error}")));
            return;
        }
    };
    let channel = {
        let _entered = runtime.enter();
        ipc_channel::channel_from_std::<ProxyCredentialRequest, ProxyCredentialResponse>(stream)
    };
    let (sender, receiver) = match channel {
        Ok(channel) => channel,
        Err(error) => {
            let _ = startup.send(Err(format!("open proxy credential channel: {error}")));
            return;
        }
    };
    if startup.send(Ok(())).is_err() {
        return;
    }
    let reason = runtime.block_on(run_actor(sender, receiver, inbox));
    let _ = closed.send(Some(reason));
}

async fn run_actor(
    sender: ipc_channel::Sender<ProxyCredentialRequest>,
    receiver: ipc_channel::Receiver<ProxyCredentialResponse>,
    mut commands: tokio::sync::mpsc::Receiver<Command>,
) -> ProxyChannelCloseReason {
    let mut request_id = 1_u64;
    loop {
        tokio::select! {
            unsolicited = receiver.recv() => {
                return match unsolicited {
                    Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => ProxyChannelCloseReason::Disconnected,
                    _ => ProxyChannelCloseReason::ProtocolError,
                };
            }
            command = commands.recv() => {
                let Some(command) = command else {
                    return ProxyChannelCloseReason::Revoked;
                };
                if dispatch(&sender, &receiver, request_id, command).await.is_err() {
                    fail_pending(&mut commands, "proxy credential capability closed");
                    return ProxyChannelCloseReason::ProtocolError;
                }
                let Some(next) = request_id.checked_add(1) else {
                    fail_pending(&mut commands, "proxy credential request id exhausted");
                    return ProxyChannelCloseReason::ProtocolError;
                };
                request_id = next;
            }
        }
    }
}

async fn dispatch(
    sender: &ipc_channel::Sender<ProxyCredentialRequest>,
    receiver: &ipc_channel::Receiver<ProxyCredentialResponse>,
    request_id: u64,
    command: Command,
) -> Result<(), String> {
    let request = request_for(request_id, &command);
    if let Err(error) = request.validate() {
        return fail(command, error.to_string());
    }
    if let Err(error) = sender.send(request).await {
        return fail(command, format!("send proxy credential request: {error}"));
    }
    let response = match tokio::time::timeout(OPERATION_TIMEOUT, receiver.recv()).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return fail(command, format!("receive proxy credential response: {error}")),
        Err(_) => return fail(command, "proxy credential response timed out".to_string()),
    };
    if response.request_id() != request_id {
        return fail(
            command,
            "proxy credential response id did not match request".to_string(),
        );
    }
    finish(command, response)
}

fn request_for(request_id: u64, command: &Command) -> ProxyCredentialRequest {
    match command {
        Command::Capture { observation, .. } => ProxyCredentialRequest::Capture {
            request_id,
            observation: wire_observation(observation),
        },
        Command::Substitute {
            domain,
            ai_provider,
            headers,
            query,
            ..
        } => ProxyCredentialRequest::Substitute {
            request_id,
            domain: domain.clone(),
            ai_provider: ai_provider.map(wire_model_provider),
            headers: wire_headers(headers),
            query: query.clone(),
        },
    }
}

fn finish(command: Command, response: ProxyCredentialResponse) -> Result<(), String> {
    match (command, response) {
        (Command::Capture { reply, .. }, ProxyCredentialResponse::Captured { credential, .. }) => {
            let _ = reply.send(Ok(core_credential(credential)));
            Ok(())
        }
        (
            Command::Substitute { reply, .. },
            ProxyCredentialResponse::Substituted {
                headers,
                query,
                credential_ref,
                ..
            },
        ) => {
            let result =
                core_headers(headers).map(|headers| (headers, BrokeredUpstreamCredentials { credential_ref, query }));
            let _ = reply.send(result);
            Ok(())
        }
        (Command::Capture { reply, .. }, ProxyCredentialResponse::Rejected { error, .. }) => {
            let _ = reply.send(Err(error));
            Ok(())
        }
        (Command::Substitute { reply, .. }, ProxyCredentialResponse::Rejected { error, .. }) => {
            let _ = reply.send(Err(error));
            Ok(())
        }
        (command, _) => fail(
            command,
            "proxy credential response kind did not match request".to_string(),
        ),
    }
}

fn fail<T>(command: Command, error: String) -> Result<T, String> {
    command.fail(error.clone());
    Err(error)
}

fn fail_pending(commands: &mut tokio::sync::mpsc::Receiver<Command>, error: &str) {
    while let Ok(command) = commands.try_recv() {
        command.fail(error.to_string());
    }
}

fn wire_observation(observation: &CredentialObservation) -> ProxyCredentialObservation {
    ProxyCredentialObservation {
        provider: wire_provider(observation.provider),
        raw_value: observation.raw_value.clone(),
        source: observation.source.clone(),
        event_type: observation.event_type.clone(),
        trace_id: observation.trace_id.clone(),
        context_json: observation.context_json.clone(),
    }
}

const fn wire_provider(provider: CredentialProvider) -> ProxyCredentialProvider {
    match provider {
        CredentialProvider::Anthropic => ProxyCredentialProvider::Anthropic,
        CredentialProvider::Google => ProxyCredentialProvider::Google,
        CredentialProvider::OpenAi => ProxyCredentialProvider::OpenAi,
        CredentialProvider::Github => ProxyCredentialProvider::Github,
        CredentialProvider::Mcp => ProxyCredentialProvider::Mcp,
    }
}

const fn core_provider(provider: ProxyCredentialProvider) -> CredentialProvider {
    match provider {
        ProxyCredentialProvider::Anthropic => CredentialProvider::Anthropic,
        ProxyCredentialProvider::Google => CredentialProvider::Google,
        ProxyCredentialProvider::OpenAi => CredentialProvider::OpenAi,
        ProxyCredentialProvider::Github => CredentialProvider::Github,
        ProxyCredentialProvider::Mcp => CredentialProvider::Mcp,
    }
}

const fn wire_model_provider(provider: ProviderKind) -> ProxyModelProvider {
    match provider {
        ProviderKind::Unknown => ProxyModelProvider::Unknown,
        ProviderKind::Anthropic => ProxyModelProvider::Anthropic,
        ProviderKind::OpenAi => ProxyModelProvider::OpenAi,
        ProviderKind::Google => ProxyModelProvider::Google,
        ProviderKind::Ollama => ProxyModelProvider::Ollama,
    }
}

fn core_credential(credential: ProxyBrokeredCredential) -> BrokeredCredential {
    BrokeredCredential {
        provider: core_provider(credential.provider),
        credential_ref: credential.credential_ref,
        store_account: credential.store_account,
        newly_captured: credential.newly_captured,
    }
}

fn wire_headers(headers: &http::HeaderMap) -> Vec<ProxyHeader> {
    headers
        .iter()
        .map(|(name, value)| ProxyHeader::new(name.as_str(), value.as_bytes().to_vec()))
        .collect()
}

fn core_headers(headers: Vec<ProxyHeader>) -> Result<http::HeaderMap, String> {
    let mut result = http::HeaderMap::new();
    for header in headers {
        let name = http::header::HeaderName::from_bytes(header.name.as_bytes())
            .map_err(|error| format!("invalid proxy credential header name: {error}"))?;
        let value = http::header::HeaderValue::from_bytes(&header.value)
            .map_err(|error| format!("invalid proxy credential header value: {error}"))?;
        result.append(name, value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
