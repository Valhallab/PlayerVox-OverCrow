use std::{
    collections::BTreeSet,
    env,
    fs::File,
    io::Read,
    os::fd::{AsRawFd, OwnedFd, RawFd},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use rustix::fs::{FileType, Mode, OFlags};
use rustix::net::Shutdown;
use tokio::{io::unix::AsyncFd, sync::oneshot};
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xproto::{ConnectionExt as _, GrabMode, Keycode, Keysym, ModMask, Window},
    },
    reexports::x11rb_protocol::{
        parse_display::{ConnectAddress, ParsedDisplay, parse_display},
        xauth::Family,
    },
    rust_connection::{DefaultStream, RustConnection},
};

use crate::{
    ShortcutError, ShortcutEvent, ShortcutFuture, ShortcutPortal, ShortcutSession, portal_trigger,
    shortcut::ShortcutDefinition,
};

const SESSION_HANDLE: &str = "x11-native";
const MIT_MAGIC_COOKIE: &[u8] = b"MIT-MAGIC-COOKIE-1";
const MAX_NATIVE_SHORTCUTS: usize = 3;
const MAX_XAUTHORITY_BYTES: u64 = 256 * 1024;
const SETUP_TIMEOUT: Duration = Duration::from_secs(5);
static AUTH_WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
const MODIFIER_BITS: [u16; 8] = [
    1 << 0,
    1 << 1,
    1 << 2,
    1 << 3,
    1 << 4,
    1 << 5,
    1 << 6,
    1 << 7,
];
const CORE_MODIFIER_MASK: u16 = 0x00ff;
const XK_CAPS_LOCK: Keysym = 0xffe5;
const XK_SHIFT_L: Keysym = 0xffe1;
const XK_SHIFT_R: Keysym = 0xffe2;
const XK_CONTROL_L: Keysym = 0xffe3;
const XK_CONTROL_R: Keysym = 0xffe4;
const XK_META_L: Keysym = 0xffe7;
const XK_META_R: Keysym = 0xffe8;
const XK_ALT_L: Keysym = 0xffe9;
const XK_ALT_R: Keysym = 0xffea;
const XK_SUPER_L: Keysym = 0xffeb;
const XK_SUPER_R: Keysym = 0xffec;
const XK_NUM_LOCK: Keysym = 0xff7f;
const XK_SCROLL_LOCK: Keysym = 0xff14;

#[derive(Clone, Debug, Default)]
pub struct X11ShortcutProvider;

impl ShortcutPortal for X11ShortcutProvider {
    fn bind(
        &self,
        definitions: Vec<ShortcutDefinition>,
    ) -> ShortcutFuture<'static, Result<Box<dyn ShortcutSession>, ShortcutError>> {
        Box::pin(async move {
            tokio::time::timeout(SETUP_TIMEOUT, prepare_session(definitions))
                .await
                .map_err(|_| ShortcutError::new("native X11 shortcut setup timed out"))?
        })
    }
}

async fn prepare_session(
    definitions: Vec<ShortcutDefinition>,
) -> Result<Box<dyn ShortcutSession>, ShortcutError> {
    let transport = connect_local_transport().await?;
    let cancellation_socket = rustix::io::dup(&transport.stream)
        .map_err(|_| ShortcutError::new("native X11 shortcut cancellation unavailable"))?;
    let mut cancellation = SetupCancellation::new(cancellation_socket);
    let auth = load_x11_auth(&transport).await?;
    let worker =
        tokio::task::spawn_blocking(move || connect_and_grab(transport, definitions, auth));
    let prepared = worker
        .await
        .map_err(|_| ShortcutError::new("native X11 shortcut worker failed"))??;
    let connection = AsyncFd::new(X11Connection(prepared.connection))
        .map_err(|_| ShortcutError::new("native X11 shortcut event source unavailable"))?;
    cancellation.disarm();
    Ok(Box::new(X11ShortcutSession {
        connection,
        bindings: prepared.bindings,
        ignored_lock_mask: prepared.ignored_lock_mask,
        key_state: NativeKeyState::default(),
    }))
}

struct X11Transport {
    stream: DefaultStream,
    screen: usize,
    display: u16,
    family: Family,
    address: Vec<u8>,
}

async fn connect_local_transport() -> Result<X11Transport, ShortcutError> {
    let display =
        parse_display(None).map_err(|_| ShortcutError::new("native X11 display is unavailable"))?;
    let socket_path = local_socket_path(&display)
        .ok_or_else(|| ShortcutError::new("native X11 display is not local"))?;
    let socket = tokio::net::UnixStream::connect(socket_path)
        .await
        .map_err(|_| ShortcutError::new("native X11 shortcut connection unavailable"))?
        .into_std()
        .map_err(|_| ShortcutError::new("native X11 shortcut connection unavailable"))?;
    let (stream, (family, address)) = DefaultStream::from_unix_stream(socket)
        .map_err(|_| ShortcutError::new("native X11 shortcut connection unavailable"))?;
    Ok(X11Transport {
        stream,
        screen: usize::from(display.screen),
        display: display.display,
        family,
        address,
    })
}

fn local_socket_path(display: &ParsedDisplay) -> Option<String> {
    display
        .connect_instruction()
        .find_map(|address| match address {
            ConnectAddress::Socket(path) => Some(path),
            _ => None,
        })
}

async fn load_x11_auth(transport: &X11Transport) -> Result<(Vec<u8>, Vec<u8>), ShortcutError> {
    let Some(path) = xauthority_path() else {
        return Ok((Vec::new(), Vec::new()));
    };
    let permit = AuthWorkerPermit::claim()?;
    let family = transport.family;
    let address = transport.address.clone();
    let display = transport.display;
    let (sender, receiver) = oneshot::channel();
    let worker = thread::Builder::new()
        .name("overcrow-x11-auth".to_owned())
        .spawn(move || {
            let _permit = permit;
            let _ = sender.send(read_x11_auth(path, family, &address, display));
        })
        .map_err(|_| ShortcutError::new("native X11 authentication worker unavailable"))?;
    // The bounded worker count prevents retries from accumulating if the
    // backing filesystem wedges. A detached OS thread cannot retain Tokio
    // shutdown, and it never owns the X11 transport or any grabs.
    drop(worker);
    receiver
        .await
        .map_err(|_| ShortcutError::new("native X11 authentication worker failed"))?
}

struct AuthWorkerPermit;

impl AuthWorkerPermit {
    fn claim() -> Result<Self, ShortcutError> {
        AUTH_WORKER_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| ShortcutError::new("native X11 authentication worker is busy"))
    }
}

impl Drop for AuthWorkerPermit {
    fn drop(&mut self) {
        AUTH_WORKER_ACTIVE.store(false, Ordering::Release);
    }
}

fn read_x11_auth(
    path: PathBuf,
    family: Family,
    address: &[u8],
    display: u16,
) -> Result<(Vec<u8>, Vec<u8>), ShortcutError> {
    let authority = match rustix::fs::open(
        &path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(authority) => authority,
        Err(_) => return Ok((Vec::new(), Vec::new())),
    };
    let metadata = rustix::fs::fstat(&authority)
        .map_err(|_| ShortcutError::new("native X11 authentication metadata unavailable"))?;
    let size = u64::try_from(metadata.st_size).unwrap_or(u64::MAX);
    if !FileType::from_raw_mode(metadata.st_mode).is_file() || size > MAX_XAUTHORITY_BYTES {
        return Err(ShortcutError::new(
            "native X11 authentication file is invalid",
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or_default());
    File::from(authority)
        .take(MAX_XAUTHORITY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ShortcutError::new("native X11 authentication file is unreadable"))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_XAUTHORITY_BYTES {
        return Err(ShortcutError::new(
            "native X11 authentication file is invalid",
        ));
    }
    parse_xauthority(&bytes, family, address, display)
}

fn xauthority_path() -> Option<PathBuf> {
    env::var_os("XAUTHORITY")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".Xauthority")))
}

fn parse_xauthority(
    bytes: &[u8],
    family: Family,
    address: &[u8],
    display: u16,
) -> Result<(Vec<u8>, Vec<u8>), ShortcutError> {
    let display = display.to_string();
    let mut offset = 0;
    while offset < bytes.len() {
        let entry_family = Family::from(read_xauthority_u16(bytes, &mut offset)?);
        let entry_address = read_xauthority_field(bytes, &mut offset)?;
        let entry_display = read_xauthority_field(bytes, &mut offset)?;
        let name = read_xauthority_field(bytes, &mut offset)?;
        let data = read_xauthority_field(bytes, &mut offset)?;
        let family_matches =
            family == Family::WILD || entry_family == Family::WILD || family == entry_family;
        if family_matches
            && (family == Family::WILD || entry_family == Family::WILD || entry_address == address)
            && (entry_display.is_empty() || entry_display == display.as_bytes())
            && name == MIT_MAGIC_COOKIE
        {
            return Ok((name.to_vec(), data.to_vec()));
        }
    }
    Ok((Vec::new(), Vec::new()))
}

fn read_xauthority_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, ShortcutError> {
    let value = bytes
        .get(*offset..offset.saturating_add(2))
        .and_then(|value| <[u8; 2]>::try_from(value).ok())
        .ok_or_else(|| ShortcutError::new("native X11 authentication file is malformed"))?;
    *offset += 2;
    Ok(u16::from_be_bytes(value))
}

fn read_xauthority_field<'a>(
    bytes: &'a [u8],
    offset: &mut usize,
) -> Result<&'a [u8], ShortcutError> {
    let length = usize::from(read_xauthority_u16(bytes, offset)?);
    let end = offset
        .checked_add(length)
        .ok_or_else(|| ShortcutError::new("native X11 authentication file is malformed"))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| ShortcutError::new("native X11 authentication file is malformed"))?;
    *offset = end;
    Ok(value)
}

struct X11Connection(RustConnection);

impl AsRawFd for X11Connection {
    fn as_raw_fd(&self) -> RawFd {
        self.0.stream().as_raw_fd()
    }
}

struct X11ShortcutSession {
    connection: AsyncFd<X11Connection>,
    bindings: Vec<NativeBinding>,
    ignored_lock_mask: u16,
    key_state: NativeKeyState,
}

impl ShortcutSession for X11ShortcutSession {
    fn handle(&self) -> &str {
        SESSION_HANDLE
    }

    fn next_event(&mut self) -> ShortcutFuture<'_, Result<ShortcutEvent, ShortcutError>> {
        Box::pin(async {
            loop {
                while let Some(event) = self
                    .connection
                    .get_ref()
                    .0
                    .poll_for_event()
                    .map_err(|_| ShortcutError::new("native X11 shortcut connection failed"))?
                {
                    match event {
                        Event::KeyPress(event) => {
                            if self.key_state.press(event.detail, event.time)
                                && let Some(shortcut_id) = activated_shortcut(
                                    &self.bindings,
                                    event.detail,
                                    u16::from(event.state),
                                    self.ignored_lock_mask,
                                )
                            {
                                return Ok(ShortcutEvent::Activated {
                                    session_handle: SESSION_HANDLE.to_owned(),
                                    shortcut_id: shortcut_id.to_owned(),
                                });
                            }
                        }
                        Event::KeyRelease(event) => {
                            self.key_state.release(event.detail, event.time);
                        }
                        Event::MappingNotify(_) => {
                            self.key_state.release_pending();
                            return Err(ShortcutError::new("native X11 keyboard mapping changed"));
                        }
                        Event::Error(_) => {
                            self.key_state.release_pending();
                            return Err(ShortcutError::new(
                                "native X11 shortcut connection failed",
                            ));
                        }
                        _ => self.key_state.release_pending(),
                    }
                }

                let mut ready = self.connection.readable().await.map_err(|_| {
                    ShortcutError::new("native X11 shortcut event source unavailable")
                })?;
                ready.clear_ready();
            }
        })
    }

    fn close(self: Box<Self>) -> ShortcutFuture<'static, Result<(), ShortcutError>> {
        Box::pin(async move {
            // Closing the X11 connection atomically releases every passive grab
            // owned by this client, including when shutdown itself is cancelled.
            drop(self);
            Ok(())
        })
    }
}

struct SetupCancellation {
    socket: OwnedFd,
    armed: bool,
}

impl SetupCancellation {
    fn new(socket: OwnedFd) -> Self {
        Self {
            socket,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SetupCancellation {
    fn drop(&mut self) {
        if self.armed {
            let _ = rustix::net::shutdown(&self.socket, Shutdown::Both);
        }
    }
}

struct PreparedSession {
    connection: RustConnection,
    bindings: Vec<NativeBinding>,
    ignored_lock_mask: u16,
}

fn connect_and_grab(
    transport: X11Transport,
    definitions: Vec<ShortcutDefinition>,
    auth: (Vec<u8>, Vec<u8>),
) -> Result<PreparedSession, ShortcutError> {
    if definitions.is_empty() || definitions.len() > MAX_NATIVE_SHORTCUTS {
        return Err(ShortcutError::new("native X11 shortcut request is invalid"));
    }
    let connection = RustConnection::connect_to_stream_with_auth_info(
        transport.stream,
        transport.screen,
        auth.0,
        auth.1,
    )
    .map_err(|_| ShortcutError::new("native X11 shortcut connection unavailable"))?;
    let setup = connection.setup();
    let screen = setup
        .roots
        .get(transport.screen)
        .ok_or_else(|| ShortcutError::new("native X11 screen unavailable"))?;
    let root = screen.root;
    let min_keycode = setup.min_keycode;
    let keycode_count = setup
        .max_keycode
        .checked_sub(min_keycode)
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| ShortcutError::new("native X11 keyboard mapping is invalid"))?;

    let keyboard = connection
        .get_keyboard_mapping(min_keycode, keycode_count)
        .map_err(|_| ShortcutError::new("native X11 keyboard mapping unavailable"))?
        .reply()
        .map_err(|_| ShortcutError::new("native X11 keyboard mapping unavailable"))?;
    let modifiers = connection
        .get_modifier_mapping()
        .map_err(|_| ShortcutError::new("native X11 modifier mapping unavailable"))?
        .reply()
        .map_err(|_| ShortcutError::new("native X11 modifier mapping unavailable"))?;
    let keymap = KeyboardMap::new(
        min_keycode,
        keycode_count,
        keyboard.keysyms_per_keycode,
        keyboard.keysyms,
    )?;
    let plan = plan_bindings(&definitions, &keymap, &modifiers.keycodes)?;
    install_grabs(&connection, root, &plan.registrations)?;

    Ok(PreparedSession {
        connection,
        bindings: plan.bindings,
        ignored_lock_mask: plan.ignored_lock_mask,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeBinding {
    id: &'static str,
    keycode: Keycode,
    modifiers: u16,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GrabRegistration {
    keycode: Keycode,
    modifiers: u16,
}

struct BindingPlan {
    bindings: Vec<NativeBinding>,
    registrations: Vec<GrabRegistration>,
    ignored_lock_mask: u16,
}

fn plan_bindings(
    definitions: &[ShortcutDefinition],
    keymap: &KeyboardMap,
    modifier_keycodes: &[Keycode],
) -> Result<BindingPlan, ShortcutError> {
    let modifiers = ModifierMapping::from_keycodes(keymap, modifier_keycodes)?;
    let mut bindings = Vec::with_capacity(definitions.len());
    let mut native_chords = BTreeSet::new();
    for definition in definitions {
        let parsed = parse_accelerator(&definition.accelerator)?;
        let keycode = keymap
            .keycode_for_ascii(parsed.key)
            .ok_or_else(|| ShortcutError::new("native X11 shortcut key is unavailable"))?;
        let mut required = 0;
        if parsed.logo {
            let logo = modifiers
                .logo
                .ok_or_else(|| ShortcutError::new("native X11 Meta modifier is unavailable"))?;
            if parsed.alt && modifiers.alt == Some(logo) {
                return Err(ShortcutError::new(
                    "native X11 modifier mapping is ambiguous",
                ));
            }
            required |= logo;
        }
        if parsed.control {
            required |= u16::from(ModMask::CONTROL);
        }
        if parsed.alt {
            required |= modifiers
                .alt
                .ok_or_else(|| ShortcutError::new("native X11 Alt modifier is unavailable"))?;
        }
        if parsed.shift {
            required |= u16::from(ModMask::SHIFT);
        }
        if required & modifiers.ignored_lock_mask != 0 {
            return Err(ShortcutError::new(
                "native X11 modifier mapping is ambiguous",
            ));
        }
        if !native_chords.insert((keycode, required)) {
            return Err(ShortcutError::new(
                "native X11 shortcut accelerators resolve to the same key",
            ));
        }
        bindings.push(NativeBinding {
            id: definition.id,
            keycode,
            modifiers: required,
        });
    }

    let lock_variants = lock_variants(&modifiers.lock_masks);
    let registrations = bindings
        .iter()
        .flat_map(|binding| {
            lock_variants.iter().map(|locks| GrabRegistration {
                keycode: binding.keycode,
                modifiers: binding.modifiers | locks,
            })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    Ok(BindingPlan {
        bindings,
        registrations,
        ignored_lock_mask: modifiers.ignored_lock_mask,
    })
}

struct ParsedAccelerator {
    logo: bool,
    control: bool,
    alt: bool,
    shift: bool,
    key: u8,
}

fn parse_accelerator(accelerator: &str) -> Result<ParsedAccelerator, ShortcutError> {
    let trigger = portal_trigger(accelerator)?;
    let mut parsed = ParsedAccelerator {
        logo: false,
        control: false,
        alt: false,
        shift: false,
        key: 0,
    };
    let mut parts = trigger.split('+').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            parsed.key = part
                .as_bytes()
                .first()
                .copied()
                .filter(|_| part.len() == 1)
                .ok_or_else(invalid_accelerator)?;
            return Ok(parsed);
        }
        match part {
            "LOGO" => parsed.logo = true,
            "CTRL" => parsed.control = true,
            "ALT" => parsed.alt = true,
            "SHIFT" => parsed.shift = true,
            _ => return Err(invalid_accelerator()),
        }
    }
    Err(invalid_accelerator())
}

fn invalid_accelerator() -> ShortcutError {
    ShortcutError::new("unsupported shortcut accelerator")
}

struct KeyboardMap {
    min_keycode: Keycode,
    keycode_count: u8,
    keysyms_per_keycode: usize,
    keysyms: Vec<Keysym>,
}

impl KeyboardMap {
    fn new(
        min_keycode: Keycode,
        keycode_count: u8,
        keysyms_per_keycode: u8,
        keysyms: Vec<Keysym>,
    ) -> Result<Self, ShortcutError> {
        let keysyms_per_keycode = usize::from(keysyms_per_keycode);
        let expected = usize::from(keycode_count)
            .checked_mul(keysyms_per_keycode)
            .ok_or_else(|| ShortcutError::new("native X11 keyboard mapping is invalid"))?;
        if keysyms_per_keycode == 0 || keysyms.len() != expected {
            return Err(ShortcutError::new("native X11 keyboard mapping is invalid"));
        }
        Ok(Self {
            min_keycode,
            keycode_count,
            keysyms_per_keycode,
            keysyms,
        })
    }

    fn keysyms(&self, keycode: Keycode) -> &[Keysym] {
        let Some(index) = keycode
            .checked_sub(self.min_keycode)
            .filter(|index| *index < self.keycode_count)
            .map(usize::from)
        else {
            return &[];
        };
        let start = index * self.keysyms_per_keycode;
        &self.keysyms[start..start + self.keysyms_per_keycode]
    }

    fn keycode_for_ascii(&self, key: u8) -> Option<Keycode> {
        let lower = u32::from(key.to_ascii_lowercase());
        let upper = u32::from(key.to_ascii_uppercase());
        (0..self.keycode_count)
            .map(|offset| self.min_keycode + offset)
            .find(|keycode| {
                self.keysyms(*keycode)
                    .iter()
                    .any(|keysym| *keysym == lower || *keysym == upper)
            })
    }
}

struct ModifierMapping {
    logo: Option<u16>,
    alt: Option<u16>,
    lock_masks: Vec<u16>,
    ignored_lock_mask: u16,
}

impl ModifierMapping {
    fn from_keycodes(keymap: &KeyboardMap, keycodes: &[Keycode]) -> Result<Self, ShortcutError> {
        if !keycodes.len().is_multiple_of(MODIFIER_BITS.len()) {
            return Err(ShortcutError::new("native X11 modifier mapping is invalid"));
        }
        let per_modifier = keycodes.len() / MODIFIER_BITS.len();
        if per_modifier == 0 {
            return Err(ShortcutError::new("native X11 modifier mapping is invalid"));
        }

        let mut super_masks = BTreeSet::new();
        let mut meta_masks = BTreeSet::new();
        let mut alt_masks = BTreeSet::new();
        let mut shift_masks = BTreeSet::new();
        let mut control_masks = BTreeSet::new();
        let mut lock_masks = BTreeSet::new();
        for (index, group) in keycodes.chunks_exact(per_modifier).enumerate() {
            let mask = MODIFIER_BITS[index];
            let keysyms = group
                .iter()
                .copied()
                .filter(|keycode| *keycode != 0)
                .flat_map(|keycode| keymap.keysyms(keycode));
            for keysym in keysyms {
                match *keysym {
                    XK_SUPER_L | XK_SUPER_R => {
                        super_masks.insert(mask);
                    }
                    XK_META_L | XK_META_R => {
                        meta_masks.insert(mask);
                    }
                    XK_ALT_L | XK_ALT_R => {
                        alt_masks.insert(mask);
                    }
                    XK_CAPS_LOCK | XK_NUM_LOCK | XK_SCROLL_LOCK => {
                        lock_masks.insert(mask);
                    }
                    XK_SHIFT_L | XK_SHIFT_R => {
                        shift_masks.insert(mask);
                    }
                    XK_CONTROL_L | XK_CONTROL_R => {
                        control_masks.insert(mask);
                    }
                    0 => {}
                    _ => {}
                }
            }
        }
        if unique_required_mask(&shift_masks)? != u16::from(ModMask::SHIFT)
            || unique_required_mask(&control_masks)? != u16::from(ModMask::CONTROL)
        {
            return Err(ShortcutError::new("native X11 modifier mapping is invalid"));
        }
        if lock_masks.iter().any(|mask| {
            shift_masks.contains(mask)
                || control_masks.contains(mask)
                || alt_masks.contains(mask)
                || super_masks.contains(mask)
                || meta_masks.contains(mask)
        }) {
            return Err(ShortcutError::new(
                "native X11 modifier mapping is ambiguous",
            ));
        }

        let logo = unique_mask(if super_masks.is_empty() {
            &meta_masks
        } else {
            &super_masks
        })?;
        let alt = unique_mask(&alt_masks)?;
        let lock_masks = lock_masks.into_iter().collect::<Vec<_>>();
        let ignored_lock_mask = lock_masks.iter().copied().fold(0, |all, mask| all | mask);
        Ok(Self {
            logo,
            alt,
            lock_masks,
            ignored_lock_mask,
        })
    }
}

fn unique_required_mask(masks: &BTreeSet<u16>) -> Result<u16, ShortcutError> {
    unique_mask(masks)?.ok_or_else(|| ShortcutError::new("native X11 modifier mapping is invalid"))
}

fn unique_mask(masks: &BTreeSet<u16>) -> Result<Option<u16>, ShortcutError> {
    if masks.len() > 1 {
        return Err(ShortcutError::new(
            "native X11 modifier mapping is ambiguous",
        ));
    }
    Ok(masks.first().copied())
}

fn lock_variants(lock_masks: &[u16]) -> Vec<u16> {
    let mut variants = BTreeSet::from([0]);
    for mask in lock_masks {
        let existing = variants.iter().copied().collect::<Vec<_>>();
        variants.extend(existing.into_iter().map(|value| value | mask));
    }
    variants.into_iter().collect()
}

fn activated_shortcut(
    bindings: &[NativeBinding],
    keycode: Keycode,
    state: u16,
    ignored_lock_mask: u16,
) -> Option<&str> {
    let modifiers = (state & CORE_MODIFIER_MASK) & !ignored_lock_mask;
    bindings
        .iter()
        .find(|binding| binding.keycode == keycode && binding.modifiers == modifiers)
        .map(|binding| binding.id)
}

#[derive(Default)]
struct NativeKeyState {
    pressed: BTreeSet<Keycode>,
    pending_release: Option<(Keycode, u32)>,
}

impl NativeKeyState {
    fn press(&mut self, keycode: Keycode, time: u32) -> bool {
        if self.pending_release == Some((keycode, time)) {
            self.pending_release = None;
            return false;
        }
        self.release_pending();
        self.pressed.insert(keycode)
    }

    fn release(&mut self, keycode: Keycode, time: u32) {
        self.release_pending();
        if self.pressed.contains(&keycode) {
            self.pending_release = Some((keycode, time));
        }
    }

    fn release_pending(&mut self) {
        if let Some((keycode, _)) = self.pending_release.take() {
            self.pressed.remove(&keycode);
        }
    }
}

trait GrabTarget {
    fn grab(&self, root: Window, registration: GrabRegistration) -> Result<(), ShortcutError>;
    fn ungrab(&self, root: Window, registration: GrabRegistration);
    fn flush(&self) -> Result<(), ShortcutError>;
}

impl GrabTarget for RustConnection {
    fn grab(&self, root: Window, registration: GrabRegistration) -> Result<(), ShortcutError> {
        self.grab_key(
            false,
            root,
            ModMask::from(registration.modifiers),
            registration.keycode,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )
        .map_err(|_| ShortcutError::new("native X11 shortcut could not be registered"))?
        .check()
        .map_err(|_| ShortcutError::new("native X11 shortcut is already in use"))
    }

    fn ungrab(&self, root: Window, registration: GrabRegistration) {
        let _ = self.ungrab_key(
            registration.keycode,
            root,
            ModMask::from(registration.modifiers),
        );
    }

    fn flush(&self) -> Result<(), ShortcutError> {
        Connection::flush(self)
            .map_err(|_| ShortcutError::new("native X11 shortcut connection failed"))
    }
}

fn install_grabs(
    target: &impl GrabTarget,
    root: Window,
    registrations: &[GrabRegistration],
) -> Result<(), ShortcutError> {
    let mut installed = Vec::with_capacity(registrations.len());
    for registration in registrations {
        if let Err(error) = target.grab(root, *registration) {
            for installed in installed.into_iter().rev() {
                target.ungrab(root, installed);
            }
            let _ = target.flush();
            return Err(error);
        }
        installed.push(*registration);
    }
    if let Err(error) = target.flush() {
        for installed in installed.into_iter().rev() {
            target.ungrab(root, installed);
        }
        let _ = target.flush();
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        io::{Read, Write},
        os::unix::net::UnixStream,
    };

    use crate::shortcut::{ShortcutAction, ShortcutDefinition};

    use super::*;

    fn definition(accelerator: &str) -> ShortcutDefinition {
        ShortcutDefinition {
            id: "test",
            description: "Test",
            accelerator: accelerator.to_owned(),
            action: ShortcutAction::ToggleOverlay,
        }
    }

    fn keyboard_map() -> KeyboardMap {
        let min = 8;
        let count = 126;
        let per_keycode = 2;
        let mut keysyms = vec![0; usize::from(count) * usize::from(per_keycode)];
        for (keycode, symbols) in [
            (24, [u32::from(b'o'), u32::from(b'O')]),
            (37, [XK_CONTROL_L, 0]),
            (50, [XK_SHIFT_L, 0]),
            (64, [XK_ALT_L, 0]),
            (66, [XK_CAPS_LOCK, 0]),
            (77, [XK_NUM_LOCK, 0]),
            (78, [XK_SCROLL_LOCK, 0]),
            (133, [XK_SUPER_L, 0]),
        ] {
            let start = usize::from(keycode - min) * usize::from(per_keycode);
            keysyms[start..start + 2].copy_from_slice(&symbols);
        }
        KeyboardMap::new(min, count, per_keycode, keysyms).expect("valid fixture keymap")
    }

    fn modifier_keycodes() -> Vec<Keycode> {
        vec![
            50, 0, // Shift
            66, 0, // Lock
            37, 0, // Control
            64, 0, // Mod1: Alt
            77, 0, // Mod2: Num Lock
            78, 0, // Mod3: Scroll Lock
            133, 0, // Mod4: Super
            0, 0, // Mod5
        ]
    }

    fn xauthority_entry(
        family: u16,
        address: &[u8],
        display: &[u8],
        name: &[u8],
        data: &[u8],
    ) -> Vec<u8> {
        let mut entry = family.to_be_bytes().to_vec();
        for field in [address, display, name, data] {
            entry.extend(
                u16::try_from(field.len())
                    .expect("bounded fixture field")
                    .to_be_bytes(),
            );
            entry.extend(field);
        }
        entry
    }

    #[test]
    fn local_display_selection_rejects_remote_x11() {
        let local = parse_display(Some(":3")).expect("local display");
        assert_eq!(
            local_socket_path(&local).as_deref(),
            Some("/tmp/.X11-unix/X3")
        );

        let remote = parse_display(Some("example.test:3")).expect("remote display");
        assert_eq!(local_socket_path(&remote), None);
    }

    #[test]
    fn xauthority_parser_is_bounded_and_fail_closed() {
        let cookie = b"0123456789abcdef";
        let matching = xauthority_entry(256, b"host", b"3", MIT_MAGIC_COOKIE, cookie);
        assert_eq!(
            parse_xauthority(&matching, Family::LOCAL, b"host", 3).expect("valid authority entry"),
            (MIT_MAGIC_COOKIE.to_vec(), cookie.to_vec())
        );

        let malformed = &matching[..matching.len() - 1];
        assert!(parse_xauthority(malformed, Family::LOCAL, b"host", 3).is_err());

        let unrelated = xauthority_entry(256, b"other", b"3", MIT_MAGIC_COOKIE, cookie);
        assert_eq!(
            parse_xauthority(&unrelated, Family::LOCAL, b"host", 3)
                .expect("well-formed unrelated entry"),
            (Vec::new(), Vec::new())
        );
    }

    #[test]
    fn xauthority_reader_accepts_only_bounded_regular_files() {
        let directory = tempfile::tempdir().expect("temporary authority directory");
        let authority = directory.path().join("authority");
        let cookie = b"0123456789abcdef";
        std::fs::write(
            &authority,
            xauthority_entry(256, b"host", b"3", MIT_MAGIC_COOKIE, cookie),
        )
        .expect("write authority fixture");
        assert_eq!(
            read_x11_auth(authority.clone(), Family::LOCAL, b"host", 3)
                .expect("bounded regular authority"),
            (MIT_MAGIC_COOKIE.to_vec(), cookie.to_vec())
        );

        std::fs::OpenOptions::new()
            .write(true)
            .open(&authority)
            .expect("open authority fixture")
            .set_len(MAX_XAUTHORITY_BYTES + 1)
            .expect("grow authority fixture");
        assert!(read_x11_auth(authority, Family::LOCAL, b"host", 3).is_err());
        assert!(read_x11_auth(directory.path().to_owned(), Family::LOCAL, b"host", 3,).is_err());
    }

    #[test]
    fn binding_plan_resolves_layout_and_registers_all_lock_variants() {
        let plan = plan_bindings(
            &[definition("Meta+Ctrl+Alt+Shift+O")],
            &keyboard_map(),
            &modifier_keycodes(),
        )
        .expect("supported X11 binding");

        assert_eq!(plan.bindings.len(), 1);
        assert_eq!(plan.bindings[0].keycode, 24);
        assert_eq!(
            plan.bindings[0].modifiers,
            u16::from(ModMask::M4)
                | u16::from(ModMask::CONTROL)
                | u16::from(ModMask::M1)
                | u16::from(ModMask::SHIFT)
        );
        assert_eq!(plan.registrations.len(), 8);
        assert_eq!(
            plan.ignored_lock_mask,
            u16::from(ModMask::LOCK) | u16::from(ModMask::M2) | u16::from(ModMask::M3)
        );
    }

    #[test]
    fn activation_ignores_lock_modifiers_but_rejects_extra_modifiers() {
        let binding = NativeBinding {
            id: "toggle",
            keycode: 24,
            modifiers: u16::from(ModMask::M4) | u16::from(ModMask::M1),
        };
        let locks = u16::from(ModMask::LOCK) | u16::from(ModMask::M2);

        assert_eq!(
            activated_shortcut(
                std::slice::from_ref(&binding),
                24,
                binding.modifiers | locks,
                locks,
            ),
            Some("toggle")
        );
        assert_eq!(
            activated_shortcut(
                &[binding],
                24,
                u16::from(ModMask::M4) | u16::from(ModMask::M1) | u16::from(ModMask::SHIFT),
                locks,
            ),
            None
        );
    }

    #[test]
    fn key_state_suppresses_x11_auto_repeat_release_press_pairs() {
        let mut state = NativeKeyState::default();

        assert!(state.press(24, 100));
        state.release(24, 600);
        assert!(!state.press(24, 600));
        state.release(24, 900);
        assert!(state.press(24, 1_100));
    }

    #[test]
    fn unavailable_or_ambiguous_modifiers_fail_closed() {
        let mut without_logo = modifier_keycodes();
        without_logo[12] = 0;
        assert!(
            plan_bindings(&[definition("Meta+Alt+O")], &keyboard_map(), &without_logo,).is_err()
        );

        let mut ambiguous_alt = modifier_keycodes();
        ambiguous_alt[14] = 64;
        assert!(
            plan_bindings(&[definition("Meta+Alt+O")], &keyboard_map(), &ambiguous_alt,).is_err()
        );
    }

    #[test]
    fn required_modifiers_cannot_share_ignored_lock_slots() {
        let mut alt_on_lock = modifier_keycodes();
        alt_on_lock[3] = 64;
        alt_on_lock[6] = 0;

        assert!(plan_bindings(&[definition("Alt+O")], &keyboard_map(), &alt_on_lock).is_err());
    }

    #[test]
    fn shift_and_control_must_use_their_core_modifier_slots() {
        let mut shifted = modifier_keycodes();
        shifted[0] = 0;
        shifted[14] = 50;
        assert!(plan_bindings(&[definition("Shift+O")], &keyboard_map(), &shifted).is_err());

        let mut controlled = modifier_keycodes();
        controlled[4] = 0;
        controlled[14] = 37;
        assert!(plan_bindings(&[definition("Ctrl+O")], &keyboard_map(), &controlled).is_err());
    }

    #[test]
    fn duplicate_native_chords_fail_closed() {
        let definitions = [
            ShortcutDefinition {
                id: "first",
                description: "First",
                accelerator: "Meta+Alt+O".to_owned(),
                action: ShortcutAction::ToggleOverlay,
            },
            ShortcutDefinition {
                id: "second",
                description: "Second",
                accelerator: "Meta+Alt+o".to_owned(),
                action: ShortcutAction::ToggleManualStopwatch,
            },
        ];

        assert!(plan_bindings(&definitions, &keyboard_map(), &modifier_keycodes()).is_err());
    }

    #[test]
    fn setup_cancellation_shutdowns_only_while_armed() {
        let (client, mut peer) = UnixStream::pair().expect("socket pair");
        peer.set_nonblocking(true).expect("nonblocking peer");
        let cancellation_socket = rustix::io::dup(&client).expect("duplicate socket");
        let cancellation = SetupCancellation::new(cancellation_socket);

        drop(cancellation);

        let mut byte = [0_u8; 1];
        assert_eq!(peer.read(&mut byte).expect("shutdown reaches peer"), 0);

        let (mut client, mut peer) = UnixStream::pair().expect("socket pair");
        let cancellation_socket = rustix::io::dup(&client).expect("duplicate socket");
        let mut cancellation = SetupCancellation::new(cancellation_socket);
        cancellation.disarm();
        drop(cancellation);
        client.write_all(b"x").expect("live socket write");
        peer.read_exact(&mut byte).expect("live socket read");
        assert_eq!(byte, *b"x");
    }

    #[derive(Default)]
    struct FakeGrabTarget {
        attempts: Cell<usize>,
        fail_at: Cell<Option<usize>>,
        installed: RefCell<Vec<GrabRegistration>>,
        released: RefCell<Vec<GrabRegistration>>,
        flushes: Cell<usize>,
    }

    impl GrabTarget for FakeGrabTarget {
        fn grab(&self, _root: Window, registration: GrabRegistration) -> Result<(), ShortcutError> {
            let attempt = self.attempts.get();
            self.attempts.set(attempt + 1);
            if self.fail_at.get() == Some(attempt) {
                return Err(ShortcutError::new("native X11 shortcut is already in use"));
            }
            self.installed.borrow_mut().push(registration);
            Ok(())
        }

        fn ungrab(&self, _root: Window, registration: GrabRegistration) {
            self.released.borrow_mut().push(registration);
        }

        fn flush(&self) -> Result<(), ShortcutError> {
            self.flushes.set(self.flushes.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn failed_registration_rolls_back_every_owned_grab() {
        let target = FakeGrabTarget::default();
        target.fail_at.set(Some(2));
        let registrations = [
            GrabRegistration {
                keycode: 24,
                modifiers: 1,
            },
            GrabRegistration {
                keycode: 24,
                modifiers: 2,
            },
            GrabRegistration {
                keycode: 24,
                modifiers: 3,
            },
        ];

        let error = install_grabs(&target, 1, &registrations).expect_err("forced conflict");

        assert_eq!(error.to_string(), "native X11 shortcut is already in use");
        assert_eq!(
            target.released.borrow().as_slice(),
            [registrations[1], registrations[0]]
        );
        assert_eq!(target.flushes.get(), 1);
    }
}
