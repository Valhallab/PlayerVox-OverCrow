use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use overcrow_config::{ShortcutSettings, WidgetProfile};
use overcrow_protocol::{CoreSnapshot, VersionedCoreSnapshot};
use tokio::sync::watch;
use zbus::{
    Address, Connection, MatchRule, MessageStream, Proxy,
    message::Type as MessageType,
    names::UniqueName,
    proxy::SignalStream,
    zvariant::{OwnedObjectPath, OwnedValue, Str},
};

use crate::CoreRuntime;

pub const SHORTCUT_ID: &str = "toggle-overlay";
pub const SHORTCUT_DESCRIPTION: &str = "Open or close the OverCrow overlay";
pub const TOGGLE_MANUAL_STOPWATCH_ID: &str = "toggle-manual-stopwatch";
pub const TOGGLE_MANUAL_STOPWATCH_DESCRIPTION: &str =
    "Start or pause the OverCrow manual stopwatch";
pub const RESET_MANUAL_STOPWATCH_ID: &str = "reset-manual-stopwatch";
pub const RESET_MANUAL_STOPWATCH_DESCRIPTION: &str = "Reset the OverCrow manual stopwatch";
// Avoid Omarchy's Super+Alt+S (move window to scratchpad).
const TOGGLE_MANUAL_STOPWATCH_ACCELERATOR: &str = "Meta+Alt+P";
const RESET_MANUAL_STOPWATCH_ACCELERATOR: &str = "Meta+Alt+R";
const REQUEST_PATH_PREFIX: &str = "/org/freedesktop/portal/desktop/request";
const SESSION_PATH_PREFIX: &str = "/org/freedesktop/portal/desktop/session/";
const PORTAL_DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const GLOBAL_SHORTCUTS_INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const HOST_PORTAL_REGISTRY_INTERFACE: &str = "org.freedesktop.host.portal.Registry";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
const SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";
const PORTAL_APP_ID: &str = "com.playervox.OverCrow";
const DBUS_DESTINATION: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";
const DBUS_INTERFACE: &str = "org.freedesktop.DBus";
const PORTAL_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_BUFFERED_REQUEST_RESPONSES: usize = 64;
pub const SHORTCUT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(not(test))]
const OWNED_CLOSE_TIMEOUT: Duration = PORTAL_CLOSE_TIMEOUT;
#[cfg(test)]
const OWNED_CLOSE_TIMEOUT: Duration = Duration::from_millis(100);

pub type ShortcutFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(crate) fn host_registry_is_unavailable(name: Option<&str>) -> bool {
    matches!(
        name,
        Some(
            "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownMethod"
        )
    )
}

pub(crate) async fn register_host_portal_identity(
    connection: &Connection,
) -> Result<(), ShortcutError> {
    let registry = Proxy::new_owned(
        connection.clone(),
        PORTAL_DESTINATION.to_owned(),
        PORTAL_PATH.to_owned(),
        HOST_PORTAL_REGISTRY_INTERFACE.to_owned(),
    )
    .await
    .map_err(portal_error)?;
    let options = HashMap::<String, OwnedValue>::new();
    match registry
        .call::<_, _, ()>("Register", &(PORTAL_APP_ID, options))
        .await
    {
        Ok(()) => Ok(()),
        Err(error) => {
            let name = match &error {
                zbus::Error::MethodError(name, _, _) => Some(name.as_str()),
                _ => None,
            };
            if host_registry_is_unavailable(name) {
                Ok(())
            } else {
                Err(portal_error(error))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutAction {
    ToggleOverlay,
    ToggleManualStopwatch,
    ResetManualStopwatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutDefinition {
    pub id: &'static str,
    pub description: &'static str,
    pub accelerator: String,
    pub action: ShortcutAction,
}

pub struct ShortcutPolicy;

impl ShortcutPolicy {
    pub fn should_bind(snapshot: &CoreSnapshot, settings: &ShortcutSettings) -> bool {
        settings.enabled && snapshot.active_game.is_some()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutError {
    message: String,
}

impl ShortcutError {
    pub fn new(message: impl fmt::Display) -> Self {
        Self {
            message: bounded_message(
                &message.to_string(),
                ShortcutAvailability::MAX_MESSAGE_BYTES,
            ),
        }
    }

    fn invalid_accelerator() -> Self {
        Self::new("unsupported shortcut accelerator")
    }
}

impl fmt::Display for ShortcutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ShortcutError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShortcutAvailability {
    Disabled,
    Binding,
    Available,
    Unavailable(String),
}

impl ShortcutAvailability {
    pub const MAX_MESSAGE_BYTES: usize = 240;
    pub const MAX_DIAGNOSTIC_BYTES: usize = Self::MAX_MESSAGE_BYTES + "unavailable: ".len();

    fn unavailable(error: impl fmt::Display) -> Self {
        Self::Unavailable(bounded_message(&error.to_string(), Self::MAX_MESSAGE_BYTES))
    }

    pub fn diagnostic(&self) -> String {
        match self {
            Self::Disabled => "disabled".to_owned(),
            Self::Binding => "binding".to_owned(),
            Self::Available => "available".to_owned(),
            Self::Unavailable(message) => format!(
                "unavailable: {}",
                bounded_message(message, Self::MAX_MESSAGE_BYTES)
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShortcutEvent {
    Activated {
        session_handle: String,
        shortcut_id: String,
    },
    Malformed,
    Closed,
    OwnerLost,
}

pub trait ShortcutSession: Send {
    fn handle(&self) -> &str;

    fn next_event(&mut self) -> ShortcutFuture<'_, Result<ShortcutEvent, ShortcutError>>;

    fn close(self: Box<Self>) -> ShortcutFuture<'static, Result<(), ShortcutError>>;
}

pub trait ShortcutPortal: Send + Sync + 'static {
    fn bind(
        &self,
        definitions: Vec<ShortcutDefinition>,
    ) -> ShortcutFuture<'static, Result<Box<dyn ShortcutSession>, ShortcutError>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResponseSource {
    Predicted,
    Any,
}

#[derive(Debug)]
pub(crate) struct PortalResponse {
    pub(crate) path: OwnedObjectPath,
    pub(crate) code: u32,
    pub(crate) results: HashMap<String, OwnedValue>,
}

#[derive(Debug)]
pub(crate) enum RequestEvent {
    Response {
        source: ResponseSource,
        response: PortalResponse,
    },
    OwnerChanged {
        old_owner: String,
        new_owner: String,
    },
}

pub(crate) trait RequestEventSource: Send {
    fn next_event(&mut self) -> ShortcutFuture<'_, Result<RequestEvent, ShortcutError>>;
}

#[derive(Clone, Debug, Default)]
pub struct XdgPortal {
    address: Option<Address>,
}

impl XdgPortal {
    pub fn for_address(address: Address) -> Self {
        Self {
            address: Some(address),
        }
    }

    async fn connect(&self) -> Result<Connection, ShortcutError> {
        match &self.address {
            Some(address) => zbus::connection::Builder::address(address.clone())
                .map_err(portal_error)?
                .build()
                .await
                .map_err(portal_error),
            None => Connection::session().await.map_err(portal_error),
        }
    }
}

impl ShortcutPortal for XdgPortal {
    fn bind(
        &self,
        definitions: Vec<ShortcutDefinition>,
    ) -> ShortcutFuture<'static, Result<Box<dyn ShortcutSession>, ShortcutError>> {
        let portal = self.clone();
        Box::pin(async move { bind_portal_session(portal, definitions).await })
    }
}

struct PortalSession {
    connection: Connection,
    handle: OwnedObjectPath,
    activations: SignalStream<'static>,
    closed: SignalStream<'static>,
    owner_changes: MessageStream,
}

impl ShortcutSession for PortalSession {
    fn handle(&self) -> &str {
        self.handle.as_str()
    }

    fn next_event(&mut self) -> ShortcutFuture<'_, Result<ShortcutEvent, ShortcutError>> {
        Box::pin(async {
            loop {
                tokio::select! {
                    message = self.activations.next() => match message {
                        Some(message) => match message.body().deserialize::<(
                            OwnedObjectPath,
                            String,
                            u64,
                            HashMap<String, OwnedValue>,
                        )>() {
                            Ok((session_handle, shortcut_id, _, _)) => return Ok(ShortcutEvent::Activated {
                                session_handle: session_handle.to_string(),
                                shortcut_id,
                            }),
                            Err(_) => return Ok(ShortcutEvent::Malformed),
                        },
                        None => return Ok(ShortcutEvent::Closed),
                    },
                    message = self.closed.next() => match message {
                        Some(message) if message.body().deserialize::<HashMap<String, OwnedValue>>().is_ok() => {
                            return Ok(ShortcutEvent::Closed);
                        }
                        Some(_) => return Ok(ShortcutEvent::Malformed),
                        None => return Ok(ShortcutEvent::Closed),
                    },
                    owner = self.owner_changes.next() => {
                        let (old_owner, new_owner) = decode_owner_change(owner)?;
                        if let Some(event) = portal_owner_change_event(&old_owner, &new_owner)? {
                            return Ok(event);
                        }
                    },
                }
            }
        })
    }

    fn close(self: Box<Self>) -> ShortcutFuture<'static, Result<(), ShortcutError>> {
        Box::pin(async move { close_portal_session(&self.connection, self.handle.clone()).await })
    }
}

impl PortalShortcutBroker<XdgPortal> {
    pub fn new(runtime: CoreRuntime) -> Self {
        Self::with_portal(runtime, XdgPortal::default())
    }
}

pub struct PortalShortcutBroker<P: ShortcutPortal> {
    runtime: CoreRuntime,
    portal: P,
    owned: OwnedBinding,
}

enum OwnedBinding {
    Idle,
    Binding {
        definitions: Vec<ShortcutDefinition>,
        future: ShortcutFuture<'static, Result<Box<dyn ShortcutSession>, ShortcutError>>,
    },
    Live {
        definitions: Vec<ShortcutDefinition>,
        session: Box<dyn ShortcutSession>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DesiredBinding {
    Disabled,
    Bind(Vec<ShortcutDefinition>),
    Invalid(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RequestPathStrategy {
    Predicted,
    Legacy(OwnedObjectPath),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControlEvent {
    Snapshot,
    Settings,
    Profile,
    Shutdown,
}

enum BrokerEvent {
    Control(ControlEvent),
    Bound(Result<Box<dyn ShortcutSession>, ShortcutError>),
    Activation(Result<ShortcutEvent, ShortcutError>),
}

impl<P: ShortcutPortal> PortalShortcutBroker<P> {
    pub fn with_portal(runtime: CoreRuntime, portal: P) -> Self {
        Self {
            runtime,
            portal,
            owned: OwnedBinding::Idle,
        }
    }

    pub fn availability(&self) -> watch::Receiver<ShortcutAvailability> {
        self.runtime.shortcut_availability()
    }

    pub async fn bind(
        &mut self,
        definitions: Vec<ShortcutDefinition>,
    ) -> Result<(), ShortcutError> {
        if let Err(error) = self.release_owned().await {
            self.publish_availability(ShortcutAvailability::unavailable(&error));
            return Err(error);
        }
        self.publish_availability(ShortcutAvailability::Binding);
        match self.portal.bind(definitions.clone()).await {
            Ok(session) => {
                self.owned = OwnedBinding::Live {
                    definitions,
                    session,
                };
                self.publish_availability(ShortcutAvailability::Available);
                Ok(())
            }
            Err(error) => {
                self.publish_availability(ShortcutAvailability::unavailable(&error));
                Err(error)
            }
        }
    }

    pub async fn close(&mut self) -> Result<(), ShortcutError> {
        let result = self.release_owned().await;
        match &result {
            Ok(()) => self.publish_availability(ShortcutAvailability::Disabled),
            Err(error) => self.publish_availability(ShortcutAvailability::unavailable(error)),
        }
        result
    }

    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) -> Result<(), ShortcutError> {
        let mut snapshots = self.runtime.snapshots();
        let mut settings = self.runtime.shortcut_settings();
        let mut profiles = self.runtime.widget_profile();
        let mut desired = desired_binding(
            &snapshots.borrow().snapshot,
            &settings.borrow(),
            &profiles.borrow(),
        );
        let mut attempted_definitions = None;

        loop {
            if *shutdown.borrow() {
                return self.close().await;
            }

            self.reconcile(&desired, &mut attempted_definitions).await;
            let event = {
                let control =
                    wait_for_control(&mut snapshots, &mut settings, &mut profiles, &mut shutdown);
                tokio::pin!(control);
                match &mut self.owned {
                    OwnedBinding::Idle => BrokerEvent::Control(control.await),
                    OwnedBinding::Binding { future, .. } => tokio::select! {
                        result = future.as_mut() => BrokerEvent::Bound(result),
                        control = &mut control => BrokerEvent::Control(control),
                    },
                    OwnedBinding::Live { session, .. } => tokio::select! {
                        result = session.next_event() => BrokerEvent::Activation(result),
                        control = &mut control => BrokerEvent::Control(control),
                    },
                }
            };

            match event {
                BrokerEvent::Control(ControlEvent::Shutdown) => {
                    return self.close().await;
                }
                BrokerEvent::Control(
                    ControlEvent::Snapshot | ControlEvent::Settings | ControlEvent::Profile,
                ) => {
                    desired = desired_binding(
                        &snapshots.borrow_and_update().snapshot,
                        &settings.borrow(),
                        &profiles.borrow(),
                    );
                }
                BrokerEvent::Bound(result) => {
                    let OwnedBinding::Binding { definitions, .. } =
                        std::mem::replace(&mut self.owned, OwnedBinding::Idle)
                    else {
                        continue;
                    };
                    match result {
                        Ok(session) => {
                            self.owned = OwnedBinding::Live {
                                definitions,
                                session,
                            };
                            self.publish_availability(ShortcutAvailability::Available);
                        }
                        Err(error) => {
                            self.publish_availability(ShortcutAvailability::unavailable(error));
                        }
                    }
                }
                BrokerEvent::Activation(Ok(ShortcutEvent::Activated {
                    session_handle,
                    shortcut_id,
                })) => {
                    let action = match &self.owned {
                        OwnedBinding::Live {
                            definitions,
                            session,
                        } if session.handle() == session_handle => definitions
                            .iter()
                            .find(|definition| definition.id == shortcut_id)
                            .map(|definition| definition.action),
                        OwnedBinding::Idle
                        | OwnedBinding::Binding { .. }
                        | OwnedBinding::Live { .. } => None,
                    };
                    match action {
                        Some(ShortcutAction::ToggleOverlay) => {
                            self.runtime.toggle_overlay().await;
                        }
                        Some(ShortcutAction::ToggleManualStopwatch) => {
                            self.runtime
                                .toggle_manual_stopwatch_at(Instant::now())
                                .await;
                        }
                        Some(ShortcutAction::ResetManualStopwatch) => {
                            self.runtime.reset_manual_stopwatch_at(Instant::now()).await;
                        }
                        None => {}
                    }
                }
                BrokerEvent::Activation(Ok(ShortcutEvent::Malformed)) => {}
                BrokerEvent::Activation(Ok(ShortcutEvent::Closed)) => {
                    self.release_after_stream_loss("portal shortcut session closed")
                        .await;
                }
                BrokerEvent::Activation(Ok(ShortcutEvent::OwnerLost)) => {
                    self.release_after_stream_loss(
                        "desktop portal owner disappeared or was replaced",
                    )
                    .await;
                }
                BrokerEvent::Activation(Err(error)) => {
                    self.release_after_stream_loss(error).await;
                }
            }
        }
    }

    async fn reconcile(
        &mut self,
        desired: &DesiredBinding,
        attempted_definitions: &mut Option<Vec<ShortcutDefinition>>,
    ) {
        let owned_matches = match (&self.owned, desired) {
            (OwnedBinding::Binding { definitions, .. }, DesiredBinding::Bind(desired))
            | (OwnedBinding::Live { definitions, .. }, DesiredBinding::Bind(desired)) => {
                definitions == desired
            }
            (OwnedBinding::Idle, _) => true,
            _ => false,
        };
        if !owned_matches && let Err(error) = self.release_owned().await {
            *attempted_definitions = match desired {
                DesiredBinding::Bind(definitions) => Some(definitions.clone()),
                DesiredBinding::Disabled | DesiredBinding::Invalid(_) => None,
            };
            self.publish_availability(ShortcutAvailability::unavailable(format_args!(
                "failed to release portal shortcut: {error}"
            )));
            return;
        }

        match desired {
            DesiredBinding::Disabled => {
                *attempted_definitions = None;
                self.publish_availability(ShortcutAvailability::Disabled);
            }
            DesiredBinding::Invalid(message) => {
                *attempted_definitions = None;
                self.publish_availability(ShortcutAvailability::unavailable(message));
            }
            DesiredBinding::Bind(definitions) => {
                if matches!(self.owned, OwnedBinding::Idle)
                    && attempted_definitions.as_ref() != Some(definitions)
                {
                    attempted_definitions.clone_from(&Some(definitions.clone()));
                    self.publish_availability(ShortcutAvailability::Binding);
                    self.owned = OwnedBinding::Binding {
                        definitions: definitions.clone(),
                        future: self.portal.bind(definitions.clone()),
                    };
                }
            }
        }
    }

    async fn release_after_stream_loss(&mut self, error: impl fmt::Display) {
        let close_error = self.release_owned().await.err();
        let message = close_error.map_or_else(
            || error.to_string(),
            |close| format!("{error}; session close failed: {close}"),
        );
        self.publish_availability(ShortcutAvailability::unavailable(message));
    }

    async fn release_owned(&mut self) -> Result<(), ShortcutError> {
        match std::mem::replace(&mut self.owned, OwnedBinding::Idle) {
            OwnedBinding::Idle | OwnedBinding::Binding { .. } => Ok(()),
            OwnedBinding::Live { session, .. } => {
                tokio::time::timeout(OWNED_CLOSE_TIMEOUT, session.close())
                    .await
                    .map_err(|_| ShortcutError::new("portal shortcut session close timed out"))?
            }
        }
    }

    fn publish_availability(&self, availability: ShortcutAvailability) {
        self.runtime.publish_shortcut_availability(availability);
    }
}

fn desired_binding(
    snapshot: &CoreSnapshot,
    settings: &ShortcutSettings,
    profile: &WidgetProfile,
) -> DesiredBinding {
    match desired_shortcuts(snapshot, settings, profile) {
        Ok(definitions) if definitions.is_empty() => DesiredBinding::Disabled,
        Ok(definitions) => DesiredBinding::Bind(definitions),
        Err(error) => DesiredBinding::Invalid(error.to_string()),
    }
}

pub(crate) fn desired_shortcuts(
    snapshot: &CoreSnapshot,
    settings: &ShortcutSettings,
    profile: &WidgetProfile,
) -> Result<Vec<ShortcutDefinition>, ShortcutError> {
    if snapshot.active_game.is_none() {
        return Ok(Vec::new());
    }

    let mut definitions = Vec::with_capacity(3);
    if settings.enabled {
        definitions.push(ShortcutDefinition {
            id: SHORTCUT_ID,
            description: SHORTCUT_DESCRIPTION,
            accelerator: settings.accelerator.clone(),
            action: ShortcutAction::ToggleOverlay,
        });
    }
    if profile.manual_stopwatch.enabled {
        for (id, description, accelerator, action) in [
            (
                TOGGLE_MANUAL_STOPWATCH_ID,
                TOGGLE_MANUAL_STOPWATCH_DESCRIPTION,
                TOGGLE_MANUAL_STOPWATCH_ACCELERATOR,
                ShortcutAction::ToggleManualStopwatch,
            ),
            (
                RESET_MANUAL_STOPWATCH_ID,
                RESET_MANUAL_STOPWATCH_DESCRIPTION,
                RESET_MANUAL_STOPWATCH_ACCELERATOR,
                ShortcutAction::ResetManualStopwatch,
            ),
        ] {
            definitions.push(ShortcutDefinition {
                id,
                description,
                accelerator: accelerator.to_owned(),
                action,
            });
        }
    }
    let mut canonical_accelerators = HashSet::with_capacity(definitions.len());
    for definition in &definitions {
        if !canonical_accelerators.insert(portal_trigger(&definition.accelerator)?) {
            return Err(ShortcutError::new(
                "shortcut accelerators must be unique within the portal session",
            ));
        }
    }
    Ok(definitions)
}

async fn wait_for_control(
    snapshots: &mut watch::Receiver<VersionedCoreSnapshot>,
    settings: &mut watch::Receiver<ShortcutSettings>,
    profiles: &mut watch::Receiver<WidgetProfile>,
    shutdown: &mut watch::Receiver<bool>,
) -> ControlEvent {
    tokio::select! {
        result = snapshots.changed() => {
            if result.is_ok() { ControlEvent::Snapshot } else { ControlEvent::Shutdown }
        }
        result = settings.changed() => {
            if result.is_ok() { ControlEvent::Settings } else { ControlEvent::Shutdown }
        }
        result = profiles.changed() => {
            if result.is_ok() { ControlEvent::Profile } else { ControlEvent::Shutdown }
        }
        result = shutdown.changed() => {
            if result.is_ok() && !*shutdown.borrow() {
                ControlEvent::Settings
            } else {
                ControlEvent::Shutdown
            }
        }
    }
}

fn bounded_message(message: &str, max_bytes: usize) -> String {
    if message.len() <= max_bytes {
        return message.to_owned();
    }
    let mut end = max_bytes;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_owned()
}

pub(crate) fn random_portal_token(kind: &str) -> Result<String, ShortcutError> {
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(ShortcutError::new("invalid portal token prefix"));
    }

    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|error| ShortcutError::new(format_args!("OS randomness unavailable: {error}")))?;
    let mut token = format!("overcrow_{kind}_");
    for byte in random {
        use fmt::Write as _;
        write!(&mut token, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(token)
}

pub(crate) fn request_path(
    unique_name: &str,
    token: &str,
) -> Result<OwnedObjectPath, ShortcutError> {
    portal_object_path(REQUEST_PATH_PREFIX, unique_name, token)
}

pub(crate) fn session_path(
    unique_name: &str,
    token: &str,
) -> Result<OwnedObjectPath, ShortcutError> {
    portal_object_path(
        SESSION_PATH_PREFIX.trim_end_matches('/'),
        unique_name,
        token,
    )
}

fn portal_object_path(
    prefix: &str,
    unique_name: &str,
    token: &str,
) -> Result<OwnedObjectPath, ShortcutError> {
    let unique_name = UniqueName::try_from(unique_name)
        .map_err(|error| ShortcutError::new(format_args!("invalid D-Bus unique name: {error}")))?;
    if !token_is_valid(token) {
        return Err(ShortcutError::new("invalid portal handle token"));
    }
    let sender = unique_name
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    format!("{prefix}/{sender}/{token}")
        .try_into()
        .map_err(|error| ShortcutError::new(format_args!("invalid portal object path: {error}")))
}

pub(crate) fn request_path_strategy(
    predicted: &OwnedObjectPath,
    returned: &str,
) -> Result<RequestPathStrategy, ShortcutError> {
    let returned: OwnedObjectPath = returned.try_into().map_err(|error| {
        ShortcutError::new(format_args!("invalid returned request path: {error}"))
    })?;
    if &returned == predicted {
        Ok(RequestPathStrategy::Predicted)
    } else {
        Ok(RequestPathStrategy::Legacy(returned))
    }
}

pub(crate) fn parse_create_response(
    response: u32,
    results: &HashMap<String, OwnedValue>,
    expected_session: &OwnedObjectPath,
) -> Result<OwnedObjectPath, ShortcutError> {
    ensure_success_response(response)?;
    let value = results
        .get("session_handle")
        .ok_or_else(|| ShortcutError::new("portal response omitted session_handle"))?;
    let handle = <&str>::try_from(value).map_err(|error| {
        ShortcutError::new(format_args!("invalid session_handle type: {error}"))
    })?;
    if handle != expected_session.as_str() {
        return Err(ShortcutError::new(
            "portal returned a session path other than the requested session",
        ));
    }
    handle
        .try_into()
        .map_err(|error| ShortcutError::new(format_args!("invalid portal session path: {error}")))
}

pub(crate) fn parse_bind_response(
    response: u32,
    shortcuts: Option<&[(String, HashMap<String, OwnedValue>)]>,
    requested_ids: &[&str],
) -> Result<(), ShortcutError> {
    ensure_success_response(response)?;
    let shortcuts =
        shortcuts.ok_or_else(|| ShortcutError::new("portal response omitted bound shortcuts"))?;
    let requested = requested_ids.iter().copied().collect::<HashSet<_>>();
    let bound = shortcuts
        .iter()
        .map(|(id, _)| id.as_str())
        .collect::<HashSet<_>>();
    if requested.len() == requested_ids.len()
        && bound.len() == shortcuts.len()
        && bound == requested
    {
        Ok(())
    } else {
        Err(ShortcutError::new(
            "portal did not bind the requested shortcut",
        ))
    }
}

pub(crate) fn parse_bind_results(
    response: u32,
    mut results: HashMap<String, OwnedValue>,
    requested_ids: &[&str],
) -> Result<(), ShortcutError> {
    ensure_success_response(response)?;
    let shortcuts: Vec<(String, HashMap<String, OwnedValue>)> = results
        .remove("shortcuts")
        .ok_or_else(|| ShortcutError::new("portal response omitted bound shortcuts"))?
        .try_into()
        .map_err(|error| {
            ShortcutError::new(format_args!("malformed bound shortcuts result: {error}"))
        })?;
    parse_bind_response(response, Some(&shortcuts), requested_ids)
}

fn ensure_success_response(response: u32) -> Result<(), ShortcutError> {
    if response == 0 {
        Ok(())
    } else {
        Err(ShortcutError::new(format_args!(
            "portal request failed with response code {response}"
        )))
    }
}

fn token_is_valid(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

async fn bind_portal_session(
    portal: XdgPortal,
    definitions: Vec<ShortcutDefinition>,
) -> Result<Box<dyn ShortcutSession>, ShortcutError> {
    let connection = portal.connect().await?;
    // Subscribe before registration so portal activation and later owner replacement are
    // observed without a race.
    let mut owner_changes = portal_owner_change_stream(&connection).await?;
    register_host_portal_identity(&connection).await?;
    let global = Proxy::new_owned(
        connection.clone(),
        PORTAL_DESTINATION.to_owned(),
        PORTAL_PATH.to_owned(),
        GLOBAL_SHORTCUTS_INTERFACE.to_owned(),
    )
    .await
    .map_err(portal_error)?;

    let create_token = random_portal_token("create")?;
    let session_token = random_portal_token("session")?;
    let unique_name = connection
        .unique_name()
        .ok_or_else(|| ShortcutError::new("portal connection has no unique D-Bus name"))?;
    let expected_session = session_path(unique_name.as_str(), &session_token)?;
    let create_options = HashMap::from([
        (
            "handle_token".to_owned(),
            OwnedValue::from(Str::from(create_token.as_str())),
        ),
        (
            "session_handle_token".to_owned(),
            OwnedValue::from(Str::from(session_token.as_str())),
        ),
    ]);
    let create_args = (create_options,);
    let create_call = global.call::<_, _, OwnedObjectPath>("CreateSession", &create_args);
    let (response, results) =
        await_request_response(&connection, &mut owner_changes, &create_token, async {
            create_call.await.map_err(portal_error)
        })
        .await?;
    let session_handle = parse_create_response(response, &results, &expected_session)?;

    let mut activations = global
        .receive_signal("Activated")
        .await
        .map_err(portal_error)?;
    // Ensure the activation stream is established before BindShortcuts can complete.
    let _ = &mut activations;
    let session_proxy = Proxy::new_owned(
        connection.clone(),
        PORTAL_DESTINATION.to_owned(),
        session_handle.clone(),
        SESSION_INTERFACE.to_owned(),
    )
    .await
    .map_err(portal_error)?;
    let closed = session_proxy
        .receive_signal("Closed")
        .await
        .map_err(portal_error)?;

    let bind_token = random_portal_token("bind")?;
    let mut shortcuts = Vec::with_capacity(definitions.len());
    for definition in &definitions {
        let trigger = portal_trigger(&definition.accelerator)?;
        let properties = HashMap::from([
            (
                "description".to_owned(),
                OwnedValue::from(Str::from(definition.description)),
            ),
            (
                "preferred_trigger".to_owned(),
                OwnedValue::from(Str::from(trigger.as_str())),
            ),
        ]);
        shortcuts.push((definition.id.to_owned(), properties));
    }
    let bind_options = HashMap::from([(
        "handle_token".to_owned(),
        OwnedValue::from(Str::from(bind_token.as_str())),
    )]);
    let bind_args = (session_handle.clone(), shortcuts, "", bind_options);
    let bind_call = global.call::<_, _, OwnedObjectPath>("BindShortcuts", &bind_args);
    let (response, results) =
        await_request_response(&connection, &mut owner_changes, &bind_token, async {
            bind_call.await.map_err(portal_error)
        })
        .await?;
    let requested_ids = definitions
        .iter()
        .map(|definition| definition.id)
        .collect::<Vec<_>>();
    parse_bind_results(response, results, &requested_ids)?;

    Ok(Box::new(PortalSession {
        connection,
        handle: session_handle,
        activations,
        closed,
        owner_changes,
    }))
}

struct PortalRequestEvents<'a> {
    predicted: MessageStream,
    any: MessageStream,
    owner_changes: &'a mut MessageStream,
}

impl RequestEventSource for PortalRequestEvents<'_> {
    fn next_event(&mut self) -> ShortcutFuture<'_, Result<RequestEvent, ShortcutError>> {
        Box::pin(async {
            tokio::select! {
                message = self.predicted.next() => {
                    decode_request_message(message, ResponseSource::Predicted)
                }
                message = self.any.next() => {
                    decode_request_message(message, ResponseSource::Any)
                }
                owner = self.owner_changes.next() => {
                    let (old_owner, new_owner) = decode_owner_change(owner)?;
                    Ok(RequestEvent::OwnerChanged { old_owner, new_owner })
                },
            }
        })
    }
}

async fn portal_owner_change_stream(
    connection: &Connection,
) -> Result<MessageStream, ShortcutError> {
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender(DBUS_DESTINATION)
        .map_err(portal_error)?
        .path(DBUS_PATH)
        .map_err(portal_error)?
        .interface(DBUS_INTERFACE)
        .map_err(portal_error)?
        .member("NameOwnerChanged")
        .map_err(portal_error)?
        .add_arg(PORTAL_DESTINATION)
        .map_err(portal_error)?
        .build();
    MessageStream::for_match_rule(rule, connection, Some(16))
        .await
        .map_err(portal_error)
}

fn decode_owner_change(
    message: Option<zbus::Result<zbus::message::Message>>,
) -> Result<(String, String), ShortcutError> {
    let message = message
        .ok_or_else(|| ShortcutError::new("portal owner watch closed"))?
        .map_err(portal_error)?;
    let (name, old_owner, new_owner) = message
        .body()
        .deserialize::<(String, String, String)>()
        .map_err(|error| {
            ShortcutError::new(format_args!("malformed portal owner change: {error}"))
        })?;
    if name != PORTAL_DESTINATION {
        return Err(ShortcutError::new(
            "portal owner watch received an unrelated bus name",
        ));
    }
    Ok((old_owner, new_owner))
}

pub(crate) fn portal_owner_change_event(
    old_owner: &str,
    new_owner: &str,
) -> Result<Option<ShortcutEvent>, ShortcutError> {
    let old_owner = parse_optional_unique_name(old_owner)?;
    let new_owner = parse_optional_unique_name(new_owner)?;
    match (old_owner, new_owner) {
        (None, Some(_)) => Ok(None),
        (Some(_), None) => Ok(Some(ShortcutEvent::OwnerLost)),
        (Some(old), Some(new)) if old != new => Ok(Some(ShortcutEvent::OwnerLost)),
        _ => Err(ShortcutError::new("malformed portal owner transition")),
    }
}

fn parse_optional_unique_name(value: &str) -> Result<Option<UniqueName<'_>>, ShortcutError> {
    if value.is_empty() {
        return Ok(None);
    }
    UniqueName::try_from(value).map(Some).map_err(|error| {
        ShortcutError::new(format_args!("invalid portal D-Bus owner name: {error}"))
    })
}

fn decode_request_message(
    message: Option<zbus::Result<zbus::message::Message>>,
    source: ResponseSource,
) -> Result<RequestEvent, ShortcutError> {
    let message = message
        .ok_or_else(|| ShortcutError::new("portal request response stream closed"))?
        .map_err(portal_error)?;
    let path = message
        .header()
        .path()
        .ok_or_else(|| ShortcutError::new("portal response omitted its object path"))?
        .to_owned();
    let (code, results) = message
        .body()
        .deserialize::<(u32, HashMap<String, OwnedValue>)>()
        .map_err(|error| ShortcutError::new(format_args!("malformed portal response: {error}")))?;
    Ok(RequestEvent::Response {
        source,
        response: PortalResponse {
            path: path.into(),
            code,
            results,
        },
    })
}

pub(crate) async fn await_monitored_response<S, F>(
    mut source: S,
    predicted: OwnedObjectPath,
    call: F,
) -> Result<(u32, HashMap<String, OwnedValue>), ShortcutError>
where
    S: RequestEventSource,
    F: Future<Output = Result<OwnedObjectPath, ShortcutError>> + Send,
{
    let mut call = Box::pin(call);
    let mut returned = None;
    let mut buffered = Vec::new();

    loop {
        if let Some(returned) = returned.as_ref()
            && let Some(index) = buffered.iter().position(|(source, response)| {
                response_matches(*source, response, &predicted, returned)
            })
        {
            let (_, response) = buffered.swap_remove(index);
            return Ok((response.code, response.results));
        }

        let event = if returned.is_some() {
            source.next_event().await?
        } else {
            tokio::select! {
                result = call.as_mut() => {
                    returned = Some(result?);
                    continue;
                }
                event = source.next_event() => event?,
            }
        };

        match event {
            RequestEvent::OwnerChanged {
                old_owner,
                new_owner,
            } => {
                if portal_owner_change_event(&old_owner, &new_owner)?.is_some() {
                    return Err(ShortcutError::new(
                        "desktop portal owner disappeared or was replaced",
                    ));
                }
            }
            RequestEvent::Response { source, response } => {
                if let Some(returned) = returned.as_ref() {
                    if response_matches(source, &response, &predicted, returned) {
                        return Ok((response.code, response.results));
                    }
                } else if buffered.len() < MAX_BUFFERED_REQUEST_RESPONSES {
                    buffered.push((source, response));
                } else {
                    return Err(ShortcutError::new(
                        "too many portal responses arrived before the method reply",
                    ));
                }
            }
        }
    }
}

fn response_matches(
    source: ResponseSource,
    response: &PortalResponse,
    predicted: &OwnedObjectPath,
    returned: &OwnedObjectPath,
) -> bool {
    match request_path_strategy(predicted, returned.as_str()) {
        Ok(RequestPathStrategy::Predicted) => {
            source == ResponseSource::Predicted && response.path == *predicted
        }
        Ok(RequestPathStrategy::Legacy(path)) => {
            source == ResponseSource::Any && response.path == path
        }
        Err(_) => false,
    }
}

async fn await_request_response<F>(
    connection: &Connection,
    owner_changes: &mut MessageStream,
    token: &str,
    call: F,
) -> Result<(u32, HashMap<String, OwnedValue>), ShortcutError>
where
    F: Future<Output = Result<OwnedObjectPath, ShortcutError>> + Send,
{
    let unique_name = connection
        .unique_name()
        .ok_or_else(|| ShortcutError::new("portal connection has no unique D-Bus name"))?;
    let predicted = request_path(unique_name.as_str(), token)?;
    let predicted_responses = request_response_stream(connection, Some(&predicted)).await?;
    let any_responses = request_response_stream(connection, None).await?;
    let source = PortalRequestEvents {
        predicted: predicted_responses,
        any: any_responses,
        owner_changes,
    };
    await_monitored_response(source, predicted, call).await
}

async fn request_response_stream(
    connection: &Connection,
    path: Option<&OwnedObjectPath>,
) -> Result<MessageStream, ShortcutError> {
    let mut rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender(PORTAL_DESTINATION)
        .map_err(portal_error)?
        .interface(REQUEST_INTERFACE)
        .map_err(portal_error)?
        .member("Response")
        .map_err(portal_error)?;
    if let Some(path) = path {
        rule = rule.path(path.as_str()).map_err(portal_error)?;
    }
    MessageStream::for_match_rule(rule.build(), connection, Some(128))
        .await
        .map_err(portal_error)
}

async fn close_portal_session(
    connection: &Connection,
    session_handle: OwnedObjectPath,
) -> Result<(), ShortcutError> {
    let close = async {
        let proxy = Proxy::new_owned(
            connection.clone(),
            PORTAL_DESTINATION.to_owned(),
            session_handle,
            SESSION_INTERFACE.to_owned(),
        )
        .await
        .map_err(portal_error)?;
        proxy
            .call::<_, _, ()>("Close", &())
            .await
            .map_err(portal_error)
    };
    tokio::time::timeout(PORTAL_CLOSE_TIMEOUT, close)
        .await
        .map_err(|_| ShortcutError::new("portal session close timed out"))?
}

fn portal_error(error: impl fmt::Display) -> ShortcutError {
    ShortcutError::new(format_args!("portal transport error: {error}"))
}

pub fn portal_trigger(accelerator: &str) -> Result<String, ShortcutError> {
    let mut parts = accelerator.split('+').peekable();
    let mut trigger_parts = Vec::new();
    let mut previous_rank = None;

    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            let mut characters = part.chars();
            let key = characters
                .next()
                .filter(char::is_ascii_alphanumeric)
                .filter(|_| characters.next().is_none())
                .ok_or_else(ShortcutError::invalid_accelerator)?;
            if trigger_parts.is_empty() {
                return Err(ShortcutError::invalid_accelerator());
            }
            trigger_parts.push(key.to_ascii_lowercase().to_string());
            return Ok(trigger_parts.join("+"));
        }

        let (rank, normalized) = match part {
            "Meta" => (0, "LOGO"),
            "Ctrl" => (1, "CTRL"),
            "Alt" => (2, "ALT"),
            "Shift" => (3, "SHIFT"),
            _ => return Err(ShortcutError::invalid_accelerator()),
        };
        if previous_rank.is_some_and(|previous| rank <= previous) {
            return Err(ShortcutError::invalid_accelerator());
        }
        previous_rank = Some(rank);
        trigger_parts.push(normalized.to_owned());
    }

    Err(ShortcutError::invalid_accelerator())
}
