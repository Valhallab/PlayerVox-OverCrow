use std::{
    error::Error, fmt, os::unix::fs::MetadataExt, path::Path, path::PathBuf, time::Duration,
};

use anyhow::{Context, anyhow, bail};
use serde::de::DeserializeOwned;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    time::timeout,
};

use crate::shortcut::{HyprBinding, ShortcutBackend};

pub const IPC_TIMEOUT: Duration = Duration::from_secs(2);
pub const MAX_REPLY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EVENT_LINE_BYTES: usize = 64 * 1024;
const MAX_BINDINGS: usize = 4_096;
const MAX_BINDING_FIELD_BYTES: usize = 16 * 1024;
const MAX_INSTANCE_SIGNATURE_BYTES: usize = 255;
const SHORTCUT_BACKEND_PROBE: &str = "eval local overcrow_shortcut_probe = true";
const COMPATIBILITY_ONLY_REPLY: &str = "eval is only supported with the lua config manager";
const MAX_BACKEND_RESPONSE_CHARS: usize = 192;

#[derive(Debug)]
pub struct UnsupportedShortcutBackend {
    response: String,
}

impl UnsupportedShortcutBackend {
    fn new(response: &str) -> Self {
        Self {
            response: response
                .trim()
                .chars()
                .flat_map(char::escape_default)
                .take(MAX_BACKEND_RESPONSE_CHARS)
                .collect(),
        }
    }
}

impl fmt::Display for UnsupportedShortcutBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Hyprland returned an unknown shortcut backend response: {}",
            self.response
        )
    }
}

impl Error for UnsupportedShortcutBackend {}

pub fn detect_shortcut_backend(response: &str) -> anyhow::Result<ShortcutBackend> {
    match response.trim() {
        "ok" => Ok(ShortcutBackend::Lua),
        COMPATIBILITY_ONLY_REPLY => Ok(ShortcutBackend::Compatibility),
        response => Err(UnsupportedShortcutBackend::new(response).into()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocketPaths {
    pub command: PathBuf,
    pub events: PathBuf,
}

impl SocketPaths {
    pub fn from_values(runtime: &Path, signature: &str) -> anyhow::Result<Self> {
        if !runtime.is_absolute() {
            bail!("XDG_RUNTIME_DIR must be absolute");
        }
        let canonical_runtime = runtime
            .canonicalize()
            .context("failed to resolve XDG_RUNTIME_DIR")?;
        if canonical_runtime != runtime {
            bail!("XDG_RUNTIME_DIR must be canonical and must not contain symlinks");
        }
        let runtime_metadata =
            std::fs::metadata(runtime).context("failed to inspect XDG_RUNTIME_DIR")?;
        let process_metadata = std::fs::metadata("/proc/self")
            .context("failed to determine the current process UID")?;
        if !runtime_metadata.is_dir()
            || runtime_metadata.uid() != process_metadata.uid()
            || runtime_metadata.mode() & 0o077 != 0
        {
            bail!("XDG_RUNTIME_DIR must be a private directory owned by the current user");
        }
        if signature.is_empty()
            || signature.len() > MAX_INSTANCE_SIGNATURE_BYTES
            || matches!(signature, "." | "..")
            || !signature
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            bail!("HYPRLAND_INSTANCE_SIGNATURE is not a safe path component");
        }
        let instance = runtime.join("hypr").join(signature);
        Ok(Self {
            command: instance.join(".socket.sock"),
            events: instance.join(".socket2.sock"),
        })
    }
}

#[derive(Clone, Debug)]
pub struct HyprlandIpc {
    paths: SocketPaths,
}

impl HyprlandIpc {
    pub fn new(paths: SocketPaths) -> Self {
        Self { paths }
    }

    pub async fn query<T: DeserializeOwned>(&self, command: &str) -> anyhow::Result<T> {
        let response = self.request(&format!("j/{command}")).await?;
        serde_json::from_slice(&response)
            .with_context(|| format!("Hyprland returned malformed JSON for {command}"))
    }

    pub async fn bindings(&self) -> anyhow::Result<Vec<HyprBinding>> {
        // Hyprland 0.56 can emit invalid JSON for `j/binds`, while its text
        // representation retains the fields required for safe reconciliation.
        let response = self.request("binds").await?;
        let response =
            std::str::from_utf8(&response).context("Hyprland bindings reply is not UTF-8")?;
        parse_bindings_text(response).context("Hyprland returned malformed bindings")
    }

    pub async fn dispatch(&self, command: &str) -> anyhow::Result<()> {
        let response = self.request(command).await?;
        let response =
            std::str::from_utf8(&response).context("Hyprland dispatch reply is not UTF-8")?;
        if response.trim() != "ok" {
            bail!("Hyprland rejected dispatch: {response}");
        }
        Ok(())
    }

    pub async fn shortcut_backend(&self) -> anyhow::Result<ShortcutBackend> {
        let response = self.request(SHORTCUT_BACKEND_PROBE).await?;
        let response = std::str::from_utf8(&response)
            .context("Hyprland shortcut backend reply is not UTF-8")?;
        detect_shortcut_backend(response)
    }

    pub async fn connect_events(&self) -> anyhow::Result<BufReader<UnixStream>> {
        let stream = timeout(IPC_TIMEOUT, UnixStream::connect(&self.paths.events))
            .await
            .context("timed out connecting to Hyprland event socket")?
            .with_context(|| {
                format!(
                    "failed to connect to Hyprland event socket {}",
                    self.paths.events.display()
                )
            })?;
        Ok(BufReader::new(stream))
    }

    async fn request(&self, request: &str) -> anyhow::Result<Vec<u8>> {
        timeout(IPC_TIMEOUT, async {
            let mut stream = UnixStream::connect(&self.paths.command)
                .await
                .with_context(|| {
                    format!(
                        "failed to connect to Hyprland command socket {}",
                        self.paths.command.display()
                    )
                })?;
            stream
                .write_all(request.as_bytes())
                .await
                .context("failed to write Hyprland IPC request")?;
            stream
                .shutdown()
                .await
                .context("failed to finish Hyprland IPC request")?;

            let mut response = Vec::new();
            stream
                .take((MAX_REPLY_BYTES + 1) as u64)
                .read_to_end(&mut response)
                .await
                .context("failed to read Hyprland IPC reply")?;
            if response.len() > MAX_REPLY_BYTES {
                return Err(anyhow!(
                    "Hyprland IPC reply exceeds {MAX_REPLY_BYTES} bytes"
                ));
            }
            Ok(response)
        })
        .await
        .context("Hyprland command timed out")?
    }
}

#[derive(Default)]
struct TextBinding {
    modmask: Option<u32>,
    key: Option<String>,
    description: Option<String>,
    dispatcher: Option<String>,
    arg: Option<String>,
}

impl TextBinding {
    fn finish(self) -> anyhow::Result<HyprBinding> {
        Ok(HyprBinding {
            modmask: self.modmask.context("binding has no modmask")?,
            key: self.key.context("binding has no key")?,
            description: self.description.context("binding has no description")?,
            dispatcher: self.dispatcher.context("binding has no dispatcher")?,
            arg: self.arg.context("binding has no argument")?,
        })
    }
}

fn parse_bindings_text(response: &str) -> anyhow::Result<Vec<HyprBinding>> {
    let mut bindings = Vec::new();
    let mut current = None::<TextBinding>;

    for line in response.lines() {
        if line.is_empty() {
            finish_text_binding(&mut current, &mut bindings)?;
            continue;
        }
        if !line.starts_with('\t') {
            finish_text_binding(&mut current, &mut bindings)?;
            if !line.starts_with("bind") || !line.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
                bail!("invalid binding record header");
            }
            current = Some(TextBinding::default());
            continue;
        }

        let current = current
            .as_mut()
            .context("binding field appears before its record header")?;
        let (name, value) = line[1..]
            .split_once(':')
            .context("binding field has no separator")?;
        let value = value.strip_prefix(' ').unwrap_or(value);
        if value.len() > MAX_BINDING_FIELD_BYTES || value.contains('\0') {
            bail!("binding field is invalid or too large");
        }
        match name {
            "modmask" => {
                if current.modmask.is_some() {
                    bail!("binding has duplicate modmask");
                }
                current.modmask = Some(value.parse().context("binding modmask is not a u32")?);
            }
            "key" => set_text_field(&mut current.key, value, "key")?,
            "description" => set_text_field(&mut current.description, value, "description")?,
            "dispatcher" => set_text_field(&mut current.dispatcher, value, "dispatcher")?,
            "arg" => set_text_field(&mut current.arg, value, "argument")?,
            _ => {}
        }
    }
    finish_text_binding(&mut current, &mut bindings)?;
    Ok(bindings)
}

fn set_text_field(
    field: &mut Option<String>,
    value: &str,
    name: &'static str,
) -> anyhow::Result<()> {
    if field.is_some() {
        bail!("binding has duplicate {name}");
    }
    *field = Some(value.to_owned());
    Ok(())
}

fn finish_text_binding(
    current: &mut Option<TextBinding>,
    bindings: &mut Vec<HyprBinding>,
) -> anyhow::Result<()> {
    let Some(current) = current.take() else {
        return Ok(());
    };
    if bindings.len() >= MAX_BINDINGS {
        bail!("binding count exceeds {MAX_BINDINGS}");
    }
    bindings.push(current.finish()?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt},
        net::UnixListener,
    };

    use crate::{model::HyprMonitor, shortcut::ShortcutBackend};

    use super::{
        HyprlandIpc, MAX_REPLY_BYTES, SocketPaths, detect_shortcut_backend, parse_bindings_text,
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    fn runtime_fixture(label: &str, mode: u32) -> PathBuf {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "overcrow-hyprland-{label}-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir(&root).expect("runtime fixture directory");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(mode))
            .expect("runtime fixture permissions");
        root
    }

    struct SocketFixture {
        root: PathBuf,
        paths: SocketPaths,
    }

    impl SocketFixture {
        fn new() -> Self {
            let root = runtime_fixture("ipc", 0o700);
            let paths = SocketPaths::from_values(&root, "fixture").expect("fixture paths");
            std::fs::create_dir_all(paths.command.parent().expect("command parent"))
                .expect("fixture directory");
            Self { root, paths }
        }

        fn ipc(&self) -> HyprlandIpc {
            HyprlandIpc::new(self.paths.clone())
        }

        fn command_listener(&self) -> UnixListener {
            UnixListener::bind(&self.paths.command).expect("command listener")
        }

        fn event_listener(&self) -> UnixListener {
            UnixListener::bind(&self.paths.events).expect("event listener")
        }
    }

    impl Drop for SocketFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    async fn reply_once(listener: UnixListener, reply: Vec<u8>) -> String {
        let (mut stream, _) = listener.accept().await.expect("client connects");
        let mut request = Vec::new();
        stream
            .read_to_end(&mut request)
            .await
            .expect("request reads");
        stream.write_all(&reply).await.expect("reply writes");
        stream.shutdown().await.expect("reply shuts down");
        String::from_utf8(request).expect("request is UTF-8")
    }

    #[test]
    fn resolves_only_a_safe_instance_component() {
        let runtime = runtime_fixture("paths", 0o700);
        let paths = SocketPaths::from_values(&runtime, "abc_123-DEF").expect("valid paths");
        assert_eq!(paths.command, runtime.join("hypr/abc_123-DEF/.socket.sock"));
        for invalid in ["", ".", "..", "a/b", "a\\b", "line\nbreak"] {
            assert!(SocketPaths::from_values(&runtime, invalid).is_err());
        }
        assert!(SocketPaths::from_values(&runtime, &"x".repeat(256)).is_err());
        assert!(SocketPaths::from_values(Path::new("relative"), "instance").is_err());
        std::fs::remove_dir_all(runtime).expect("runtime fixture cleanup");
    }

    #[test]
    fn rejects_runtime_directories_that_are_shared_or_not_canonical() {
        let shared = runtime_fixture("shared", 0o755);
        assert!(SocketPaths::from_values(&shared, "instance").is_err());

        let private = runtime_fixture("private", 0o700);
        let link = private.with_extension("link");
        std::os::unix::fs::symlink(&private, &link).expect("runtime symlink");
        assert!(SocketPaths::from_values(&link, "instance").is_err());
        assert!(SocketPaths::from_values(Path::new("/tmp"), "instance").is_err());

        std::fs::remove_file(link).expect("runtime symlink cleanup");
        std::fs::remove_dir_all(private).expect("private runtime cleanup");
        std::fs::remove_dir_all(shared).expect("shared runtime cleanup");
    }

    #[tokio::test]
    async fn query_sends_json_prefix_and_decodes_a_bounded_reply() {
        let fixture = SocketFixture::new();
        let listener = fixture.command_listener();
        let server = tokio::spawn(reply_once(listener, br#"[{"id":0,"scale":1.25}]"#.to_vec()));

        let monitors: Vec<HyprMonitor> =
            fixture.ipc().query("monitors").await.expect("query works");

        assert_eq!(server.await.expect("server completes"), "j/monitors");
        assert_eq!(monitors, vec![HyprMonitor { id: 0, scale: 1.25 }]);
    }

    #[tokio::test]
    async fn bindings_use_the_stable_text_reply_on_hyprland_056() {
        let fixture = SocketFixture::new();
        let listener = fixture.command_listener();
        let reply = b"bindled\n\tlocked: false\n\tmouse: false\n\trelease: false\n\trepeat: false\n\tlongPress: false\n\tmodmask: 72\n\tsubmap: \n\tkey: O\n\tkeycode: 0\n\tcatchall: false\n\tdescription: OverCrow overlay\n\tdispatcher: global\n\targ: com.playervox.OverCrow:toggle-overlay\n\n";
        let server = tokio::spawn(reply_once(listener, reply.to_vec()));

        let bindings = fixture.ipc().bindings().await.expect("bindings decode");

        assert_eq!(server.await.expect("server completes"), "binds");
        assert_eq!(
            bindings,
            vec![crate::shortcut::HyprBinding {
                modmask: 72,
                key: "O".to_owned(),
                description: "OverCrow overlay".to_owned(),
                dispatcher: "global".to_owned(),
                arg: "com.playervox.OverCrow:toggle-overlay".to_owned(),
            }]
        );
    }

    #[test]
    fn binding_text_parser_rejects_incomplete_or_ambiguous_records() {
        for malformed in [
            "unknown\n\tmodmask: 72\n",
            "bind\n\tkey: O\n\tdescription: owned\n\tdispatcher: global\n\targ: :owned\n",
            "bind\n\tmodmask: true\n\tkey: O\n\tdescription: owned\n\tdispatcher: global\n\targ: :owned\n",
            "bind\n\tmodmask: 72\n\tkey: O\n\tkey: P\n\tdescription: owned\n\tdispatcher: global\n\targ: :owned\n",
        ] {
            assert!(
                parse_bindings_text(malformed).is_err(),
                "accepted malformed binding: {malformed:?}"
            );
        }
    }

    #[tokio::test]
    async fn rejects_oversized_and_malformed_replies() {
        let fixture = SocketFixture::new();
        let listener = fixture.command_listener();
        let server = tokio::spawn(reply_once(listener, vec![b'x'; MAX_REPLY_BYTES + 1]));
        assert!(
            fixture
                .ipc()
                .query::<serde_json::Value>("clients")
                .await
                .is_err()
        );
        server.await.expect("server completes");

        let fixture = SocketFixture::new();
        let listener = fixture.command_listener();
        let server = tokio::spawn(reply_once(listener, b"{".to_vec()));
        assert!(
            fixture
                .ipc()
                .query::<serde_json::Value>("clients")
                .await
                .is_err()
        );
        server.await.expect("server completes");
    }

    #[tokio::test]
    async fn dispatch_requires_an_ok_reply() {
        let fixture = SocketFixture::new();
        let listener = fixture.command_listener();
        let server = tokio::spawn(reply_once(listener, b"ok".to_vec()));
        fixture
            .ipc()
            .dispatch("dispatch alterzorder top,address:0x20")
            .await
            .expect("ok dispatch");
        assert_eq!(
            server.await.expect("server completes"),
            "dispatch alterzorder top,address:0x20"
        );

        let fixture = SocketFixture::new();
        let listener = fixture.command_listener();
        let server = tokio::spawn(reply_once(listener, b"error: invalid dispatcher".to_vec()));
        assert!(fixture.ipc().dispatch("dispatch invalid").await.is_err());
        server.await.expect("server completes");

        let fixture = SocketFixture::new();
        let listener = fixture.command_listener();
        let server = tokio::spawn(reply_once(listener, b"ok".to_vec()));
        fixture
            .ipc()
            .dispatch("eval hl.unbind(\"SUPER + ALT + O\")")
            .await
            .expect("Lua dispatch");
        assert_eq!(
            server.await.expect("server completes"),
            "eval hl.unbind(\"SUPER + ALT + O\")"
        );
    }

    #[test]
    fn detects_only_the_two_supported_config_managers() {
        assert_eq!(detect_shortcut_backend("ok").unwrap(), ShortcutBackend::Lua);
        assert_eq!(
            detect_shortcut_backend("eval is only supported with the lua config manager").unwrap(),
            ShortcutBackend::Compatibility
        );
        assert!(detect_shortcut_backend("unexpected response").is_err());

        let hostile = format!("{}\nterminal control: \u{1b}[31m", "x".repeat(1_000));
        let error = detect_shortcut_backend(&hostile).unwrap_err();
        assert!(
            error
                .downcast_ref::<super::UnsupportedShortcutBackend>()
                .is_some()
        );
        assert!(error.to_string().len() <= 256);
        assert!(!error.to_string().contains('\n'));
    }

    #[tokio::test]
    async fn backend_probe_is_bounded_and_uses_side_effect_free_lua() {
        let fixture = SocketFixture::new();
        let listener = fixture.command_listener();
        let server = tokio::spawn(reply_once(
            listener,
            b"eval is only supported with the lua config manager".to_vec(),
        ));

        assert_eq!(
            fixture.ipc().shortcut_backend().await.unwrap(),
            ShortcutBackend::Compatibility
        );
        assert_eq!(
            server.await.unwrap(),
            "eval local overcrow_shortcut_probe = true"
        );
    }

    #[tokio::test]
    async fn connects_to_the_event_socket() {
        let fixture = SocketFixture::new();
        let listener = fixture.event_listener();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("event client connects");
            stream
                .write_all(b"activewindowv2>>55aabb\n")
                .await
                .expect("event writes");
        });

        let mut events = fixture.ipc().connect_events().await.expect("event socket");
        let mut line = String::new();
        events.read_line(&mut line).await.expect("event reads");

        assert_eq!(line, "activewindowv2>>55aabb\n");
        server.await.expect("server completes");
    }
}
