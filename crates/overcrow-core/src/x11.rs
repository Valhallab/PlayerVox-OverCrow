use anyhow::Context;
use overcrow_protocol::Rect;
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::ErrorKind;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, MapState, Window};
use x11rb::rust_connection::RustConnection;

use crate::{OVERLAY_APP_ID, WindowObservation, WindowSource};

const MAX_TEXT_PROPERTY_LENGTH: u32 = 4096;

pub struct X11WindowSource {
    backend: Box<dyn X11Backend>,
    last_non_overlay_window: Option<Window>,
}

struct LiveX11Backend {
    connection: RustConnection,
    root: Window,
    atoms: X11Atoms,
}

trait X11Backend {
    fn active_window(&mut self) -> anyhow::Result<Option<Window>>;
    fn read_window(&mut self, window: Window)
    -> Result<Option<WindowObservation>, ReadWindowError>;
}

#[derive(Debug)]
enum ReadWindowError {
    Disappeared,
    Other(anyhow::Error),
}

impl ReadWindowError {
    fn other(error: impl std::error::Error + Send + Sync + 'static, context: &'static str) -> Self {
        Self::Other(anyhow::Error::new(error).context(context))
    }
}

struct X11Atoms {
    active_window: Atom,
    pid: Atom,
    name: Atom,
    utf8_string: Atom,
    wm_class: Atom,
}

impl X11WindowSource {
    pub fn connect() -> anyhow::Result<Self> {
        let (connection, screen_number) = x11rb::connect(None)?;
        let root = connection
            .setup()
            .roots
            .get(screen_number)
            .context("X11 connection returned an invalid screen number")?
            .root;
        let atoms = X11Atoms {
            active_window: connection
                .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
                .reply()?
                .atom,
            pid: connection.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom,
            name: connection
                .intern_atom(false, b"_NET_WM_NAME")?
                .reply()?
                .atom,
            utf8_string: connection.intern_atom(false, b"UTF8_STRING")?.reply()?.atom,
            wm_class: connection.intern_atom(false, b"WM_CLASS")?.reply()?.atom,
        };

        Ok(Self::from_backend(LiveX11Backend {
            connection,
            root,
            atoms,
        }))
    }

    fn from_backend(backend: impl X11Backend + 'static) -> Self {
        Self {
            backend: Box::new(backend),
            last_non_overlay_window: None,
        }
    }

    fn read_window(&mut self, window: Window) -> anyhow::Result<Option<WindowObservation>> {
        match self.backend.read_window(window) {
            Ok(observation) => Ok(observation),
            Err(ReadWindowError::Disappeared) => Ok(None),
            Err(ReadWindowError::Other(error)) => Err(error),
        }
    }
}

impl WindowSource for X11WindowSource {
    fn active_window(&mut self) -> anyhow::Result<Option<WindowObservation>> {
        let Some(active_window) = self.backend.active_window()? else {
            self.last_non_overlay_window = None;
            return Ok(None);
        };
        let Some(active_observation) = self.read_window(active_window)? else {
            self.last_non_overlay_window = None;
            return Ok(None);
        };

        if active_observation.app_id.as_deref() != Some(OVERLAY_APP_ID) {
            self.last_non_overlay_window = Some(active_window);
            return Ok(Some(active_observation));
        }

        let Some(underlying_window) = self.last_non_overlay_window else {
            return Ok(None);
        };
        let observation = self.read_window(underlying_window)?;
        if observation.is_none() {
            self.last_non_overlay_window = None;
        }
        Ok(observation)
    }
}

impl X11Backend for LiveX11Backend {
    fn active_window(&mut self) -> anyhow::Result<Option<Window>> {
        let active_reply = self
            .connection
            .get_property(
                false,
                self.root,
                self.atoms.active_window,
                AtomEnum::WINDOW,
                0,
                1,
            )?
            .reply()?;
        Ok(active_reply
            .value32()
            .and_then(|mut values| values.next())
            .filter(|window| *window != x11rb::NONE))
    }

    fn read_window(
        &mut self,
        window: Window,
    ) -> Result<Option<WindowObservation>, ReadWindowError> {
        let attributes = self
            .connection
            .get_window_attributes(window)
            .map_err(|error| {
                ReadWindowError::other(error, "failed to request X11 window attributes")
            })?
            .reply();
        let attributes = read_reply(attributes, "failed to read X11 window attributes")?;
        if attributes.map_state != MapState::VIEWABLE {
            return Ok(None);
        }

        let pid_reply = self
            .connection
            .get_property(false, window, self.atoms.pid, AtomEnum::CARDINAL, 0, 1)
            .map_err(|error| ReadWindowError::other(error, "failed to request X11 window PID"))?
            .reply();
        let pid_reply = read_reply(pid_reply, "failed to read X11 window PID")?;
        let title = self
            .connection
            .get_property(
                false,
                window,
                self.atoms.name,
                self.atoms.utf8_string,
                0,
                MAX_TEXT_PROPERTY_LENGTH,
            )
            .map_err(|error| ReadWindowError::other(error, "failed to request X11 window title"))?
            .reply();
        let title = read_reply(title, "failed to read X11 window title")?.value;
        let wm_class = self
            .connection
            .get_property(
                false,
                window,
                self.atoms.wm_class,
                AtomEnum::STRING,
                0,
                MAX_TEXT_PROPERTY_LENGTH,
            )
            .map_err(|error| ReadWindowError::other(error, "failed to request X11 WM_CLASS"))?
            .reply();
        let wm_class = read_reply(wm_class, "failed to read X11 WM_CLASS")?.value;
        let geometry = self.connection.get_geometry(window).map_err(|error| {
            ReadWindowError::other(error, "failed to request X11 window geometry")
        })?;
        let geometry = read_reply(geometry.reply(), "failed to read X11 window geometry")?;
        let translated = self
            .connection
            .translate_coordinates(window, self.root, 0, 0)
            .map_err(|error| {
                ReadWindowError::other(error, "failed to request X11 root coordinates")
            })?;
        let translated = read_reply(translated.reply(), "failed to read X11 root coordinates")?;

        Ok(Some(normalize_observation(RawWindowObservation {
            pid: pid_reply
                .value32()
                .and_then(|mut values| values.next())
                .filter(|pid| *pid != 0),
            wm_class,
            title,
            root_x: translated.dst_x,
            root_y: translated.dst_y,
            width: geometry.width,
            height: geometry.height,
        })))
    }
}

fn read_reply<T>(
    reply: Result<T, ReplyError>,
    context: &'static str,
) -> Result<T, ReadWindowError> {
    match reply {
        Ok(reply) => Ok(reply),
        Err(ReplyError::X11Error(error)) if error.error_kind == ErrorKind::Window => {
            Err(ReadWindowError::Disappeared)
        }
        Err(error) => Err(ReadWindowError::other(error, context)),
    }
}

struct RawWindowObservation {
    pid: Option<u32>,
    wm_class: Vec<u8>,
    title: Vec<u8>,
    root_x: i16,
    root_y: i16,
    width: u16,
    height: u16,
}

fn normalize_observation(raw: RawWindowObservation) -> WindowObservation {
    WindowObservation {
        pid: raw.pid,
        app_id: normalize_wm_class(&raw.wm_class),
        title: normalize_text(&raw.title),
        rect: Rect {
            x: i32::from(raw.root_x),
            y: i32::from(raw.root_y),
            width: u32::from(raw.width),
            height: u32::from(raw.height),
        },
        scale: 1.0,
        backend: "x11".to_owned(),
    }
}

fn normalize_wm_class(value: &[u8]) -> Option<String> {
    let mut fields = value
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let instance = fields.next();
    fields
        .next()
        .or(instance)
        .map(|field| String::from_utf8_lossy(field).into_owned())
}

fn normalize_text(value: &[u8]) -> String {
    let text = value.split(|byte| *byte == 0).next().unwrap_or_default();
    String::from_utf8_lossy(text).into_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use overcrow_protocol::Rect;
    use x11rb::errors::ReplyError;
    use x11rb::protocol::ErrorKind;
    use x11rb::protocol::xproto::Window;
    use x11rb::x11_utils::X11Error;

    use super::{
        RawWindowObservation, ReadWindowError, X11Backend, X11WindowSource, normalize_observation,
        read_reply,
    };
    use crate::{WindowObservation, WindowSource};

    #[test]
    fn x11_adapter_implements_window_source_without_connecting() {
        fn assert_window_source<T: WindowSource>() {}

        assert_window_source::<X11WindowSource>();
    }

    #[test]
    fn normalizes_x11_metadata_without_a_live_display() {
        let raw = RawWindowObservation {
            pid: Some(42),
            wm_class: b"portal2\0Valve001\0".to_vec(),
            title: b"Portal 2".to_vec(),
            root_x: -1920,
            root_y: 24,
            width: 1920,
            height: 1080,
        };

        assert_eq!(
            normalize_observation(raw),
            WindowObservation {
                pid: Some(42),
                app_id: Some("Valve001".to_owned()),
                title: "Portal 2".to_owned(),
                rect: Rect {
                    x: -1920,
                    y: 24,
                    width: 1920,
                    height: 1080,
                },
                scale: 1.0,
                backend: "x11".to_owned(),
            }
        );
    }

    #[test]
    fn malformed_optional_x11_metadata_degrades_to_partial_text() {
        let raw = RawWindowObservation {
            pid: None,
            wm_class: b"\0\0".to_vec(),
            title: b"broken \xff title\0ignored".to_vec(),
            root_x: 10,
            root_y: 20,
            width: 800,
            height: 600,
        };

        let observation = normalize_observation(raw);

        assert_eq!(observation.app_id, None);
        assert_eq!(observation.title, "broken � title");
    }

    #[test]
    fn overlay_focus_reobserves_the_last_non_overlay_window() {
        const GAME: Window = 10;
        const OVERLAY: Window = 20;
        let backend = FakeBackend::new(
            [Some(GAME), Some(OVERLAY)],
            [
                (
                    GAME,
                    FakeRead::Observed(sample_observation("portal2", 100, 200)),
                ),
                (
                    OVERLAY,
                    FakeRead::Observed(sample_observation("io.github.overcrow.Overlay", 100, 200)),
                ),
                (
                    GAME,
                    FakeRead::Observed(sample_observation("portal2", 100, 200)),
                ),
            ],
        );
        let mut source = X11WindowSource::from_backend(backend);

        assert_eq!(
            source.active_window().expect("initial game observation"),
            Some(sample_observation("portal2", 100, 200))
        );
        assert_eq!(
            source.active_window().expect("underlying game refresh"),
            Some(sample_observation("portal2", 100, 200))
        );
    }

    #[test]
    fn overlay_focus_without_a_previous_window_returns_none() {
        const OVERLAY: Window = 20;
        let backend = FakeBackend::new(
            [Some(OVERLAY)],
            [(
                OVERLAY,
                FakeRead::Observed(sample_observation("io.github.overcrow.Overlay", 100, 200)),
            )],
        );
        let mut source = X11WindowSource::from_backend(backend);

        assert_eq!(source.active_window().expect("overlay read succeeds"), None);
    }

    #[test]
    fn overlay_focus_refreshes_the_underlying_window_geometry() {
        const GAME: Window = 10;
        const OVERLAY: Window = 20;
        let backend = FakeBackend::new(
            [Some(GAME), Some(OVERLAY)],
            [
                (
                    GAME,
                    FakeRead::Observed(sample_observation("portal2", 100, 200)),
                ),
                (
                    OVERLAY,
                    FakeRead::Observed(sample_observation("io.github.overcrow.Overlay", 100, 200)),
                ),
                (
                    GAME,
                    FakeRead::Observed(sample_observation("portal2", 320, 480)),
                ),
            ],
        );
        let mut source = X11WindowSource::from_backend(backend);

        source.active_window().expect("initial game observation");
        let refreshed = source
            .active_window()
            .expect("underlying game refresh")
            .expect("game remains viewable");

        assert_eq!(refreshed.rect.x, 320);
        assert_eq!(refreshed.rect.y, 480);
    }

    #[test]
    fn destroyed_underlying_window_is_forgotten() {
        underlying_window_loss_is_forgotten(FakeRead::Disappeared);
    }

    #[test]
    fn non_viewable_underlying_window_is_forgotten() {
        underlying_window_loss_is_forgotten(FakeRead::NotViewable);
    }

    #[test]
    fn non_bad_window_failure_remains_an_error() {
        const GAME: Window = 10;
        const OVERLAY: Window = 20;
        let backend = FakeBackend::new(
            [Some(GAME), Some(OVERLAY)],
            [
                (
                    GAME,
                    FakeRead::Observed(sample_observation("portal2", 100, 200)),
                ),
                (
                    OVERLAY,
                    FakeRead::Observed(sample_observation("io.github.overcrow.Overlay", 100, 200)),
                ),
                (GAME, FakeRead::Failed("connection reset")),
            ],
        );
        let mut source = X11WindowSource::from_backend(backend);

        source.active_window().expect("initial game observation");
        let error = source
            .active_window()
            .expect_err("connection failures must not look like window disappearance");

        assert!(error.to_string().contains("connection reset"));
    }

    #[test]
    fn another_active_window_replaces_the_remembered_xid() {
        const GAME: Window = 10;
        const OVERLAY: Window = 20;
        const OTHER: Window = 30;
        let backend = FakeBackend::new(
            [Some(GAME), Some(OVERLAY), Some(OTHER), Some(OVERLAY)],
            [
                (
                    GAME,
                    FakeRead::Observed(sample_observation("portal2", 100, 200)),
                ),
                (
                    OVERLAY,
                    FakeRead::Observed(sample_observation("io.github.overcrow.Overlay", 100, 200)),
                ),
                (
                    GAME,
                    FakeRead::Observed(sample_observation("portal2", 100, 200)),
                ),
                (
                    OTHER,
                    FakeRead::Observed(sample_observation("firefox", 400, 300)),
                ),
                (
                    OVERLAY,
                    FakeRead::Observed(sample_observation("io.github.overcrow.Overlay", 100, 200)),
                ),
                (
                    OTHER,
                    FakeRead::Observed(sample_observation("firefox", 410, 310)),
                ),
            ],
        );
        let mut source = X11WindowSource::from_backend(backend);

        source.active_window().expect("initial game observation");
        source.active_window().expect("game under overlay");
        assert_eq!(
            source.active_window().expect("other active window"),
            Some(sample_observation("firefox", 400, 300))
        );
        assert_eq!(
            source.active_window().expect("other window under overlay"),
            Some(sample_observation("firefox", 410, 310))
        );
    }

    #[test]
    fn bad_window_reply_is_treated_as_disappearance() {
        let result = read_reply::<()>(Err(x11_error(ErrorKind::Window)), "read failed");

        assert!(matches!(result, Err(ReadWindowError::Disappeared)));
    }

    #[test]
    fn unrelated_protocol_error_is_preserved() {
        let result = read_reply::<()>(Err(x11_error(ErrorKind::Value)), "read failed");

        let Err(ReadWindowError::Other(error)) = result else {
            panic!("non-BadWindow protocol errors must remain errors");
        };
        assert!(error.to_string().contains("read failed"));
    }

    fn x11_error(error_kind: ErrorKind) -> ReplyError {
        ReplyError::X11Error(X11Error {
            error_kind,
            error_code: 0,
            sequence: 0,
            bad_value: 0,
            minor_opcode: 0,
            major_opcode: 0,
            extension_name: None,
            request_name: None,
        })
    }

    fn underlying_window_loss_is_forgotten(loss: FakeRead) {
        const GAME: Window = 10;
        const OVERLAY: Window = 20;
        let backend = FakeBackend::new(
            [Some(GAME), Some(OVERLAY), Some(OVERLAY)],
            [
                (
                    GAME,
                    FakeRead::Observed(sample_observation("portal2", 100, 200)),
                ),
                (
                    OVERLAY,
                    FakeRead::Observed(sample_observation("io.github.overcrow.Overlay", 100, 200)),
                ),
                (GAME, loss),
                (
                    OVERLAY,
                    FakeRead::Observed(sample_observation("io.github.overcrow.Overlay", 100, 200)),
                ),
            ],
        );
        let mut source = X11WindowSource::from_backend(backend);

        source.active_window().expect("initial game observation");
        assert_eq!(source.active_window().expect("window loss is normal"), None);
        assert_eq!(
            source
                .active_window()
                .expect("forgotten window is not queried again"),
            None
        );
    }

    struct FakeBackend {
        active_windows: VecDeque<Option<Window>>,
        reads: VecDeque<(Window, FakeRead)>,
    }

    enum FakeRead {
        Observed(WindowObservation),
        NotViewable,
        Disappeared,
        Failed(&'static str),
    }

    impl FakeBackend {
        fn new(
            active_windows: impl IntoIterator<Item = Option<Window>>,
            reads: impl IntoIterator<Item = (Window, FakeRead)>,
        ) -> Self {
            Self {
                active_windows: active_windows.into_iter().collect(),
                reads: reads.into_iter().collect(),
            }
        }
    }

    impl X11Backend for FakeBackend {
        fn active_window(&mut self) -> anyhow::Result<Option<Window>> {
            Ok(self
                .active_windows
                .pop_front()
                .expect("unexpected active-window read"))
        }

        fn read_window(
            &mut self,
            window: Window,
        ) -> Result<Option<WindowObservation>, ReadWindowError> {
            let (expected_window, result) = self.reads.pop_front().expect("unexpected window read");
            assert_eq!(window, expected_window);
            match result {
                FakeRead::Observed(observation) => Ok(Some(observation)),
                FakeRead::NotViewable => Ok(None),
                FakeRead::Disappeared => Err(ReadWindowError::Disappeared),
                FakeRead::Failed(message) => Err(ReadWindowError::Other(anyhow::anyhow!(message))),
            }
        }
    }

    fn sample_observation(app_id: &str, x: i32, y: i32) -> WindowObservation {
        WindowObservation {
            pid: Some(42),
            app_id: Some(app_id.to_owned()),
            title: "Portal 2".to_owned(),
            rect: Rect {
                x,
                y,
                width: 1920,
                height: 1080,
            },
            scale: 1.0,
            backend: "x11".to_owned(),
        }
    }
}
