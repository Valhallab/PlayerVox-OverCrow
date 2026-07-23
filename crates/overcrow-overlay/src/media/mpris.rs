use std::collections::HashMap;

use zbus::{
    Connection, Proxy,
    fdo::DBusProxy,
    zvariant::{Array, OwnedValue},
};

use super::model::{
    MediaAction, MediaCapabilities, MediaCommand, MediaPlaybackStatus, MediaSnapshot,
    bound_nonempty_text,
};

const MPRIS_BUS_PREFIX: &str = "org.mpris.MediaPlayer2.";
pub(crate) const MPRIS_CANDIDATE_LIMIT: usize = 16;
pub(crate) const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
pub(crate) const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
pub(crate) const TITLE_MAX_BYTES: usize = 256;
const ARTIST_MAX_BYTES: usize = 256;
const ARTIST_MAX_COUNT: usize = 8;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MediaMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
}

pub(crate) fn bound_text(value: &str, maximum_bytes: usize) -> Option<String> {
    bound_nonempty_text(value, maximum_bytes)
}

pub(crate) fn parse_playback_status(value: &str) -> Option<MediaPlaybackStatus> {
    match value {
        "Playing" => Some(MediaPlaybackStatus::Playing),
        "Paused" => Some(MediaPlaybackStatus::Paused),
        "Stopped" => Some(MediaPlaybackStatus::Stopped),
        _ => None,
    }
}

pub(crate) fn parse_metadata(metadata: &HashMap<String, OwnedValue>) -> MediaMetadata {
    // zbus owns the already-deserialized property message. OverCrow borrows only
    // the two allowed keys and bounds every string/count copied from that message.
    let title = metadata
        .get("xesam:title")
        .and_then(|value| <&str>::try_from(value).ok())
        .and_then(|value| bound_text(value, TITLE_MAX_BYTES));
    let artist = metadata.get("xesam:artist").and_then(parse_artists);

    MediaMetadata { title, artist }
}

fn parse_artists(value: &OwnedValue) -> Option<String> {
    let artists = <&Array<'_>>::try_from(value).ok()?;
    let mut output = String::new();
    for value in artists.iter().take(ARTIST_MAX_COUNT) {
        let artist = <&str>::try_from(value).ok()?;
        if !append_artist(&mut output, artist) {
            break;
        }
    }
    (!output.is_empty()).then_some(output)
}

fn append_artist(output: &mut String, artist: &str) -> bool {
    let artist = artist.trim();
    if artist.is_empty() {
        return true;
    }

    let separator = if output.is_empty() { "" } else { ", " };
    let remaining = ARTIST_MAX_BYTES.saturating_sub(output.len());
    if remaining <= separator.len() {
        return false;
    }
    let maximum_artist_bytes = remaining - separator.len();
    let mut boundary = artist.len().min(maximum_artist_bytes);
    while boundary > 0 && !artist.is_char_boundary(boundary) {
        boundary -= 1;
    }
    if boundary == 0 {
        return false;
    }

    output.push_str(separator);
    output.push_str(&artist[..boundary]);
    boundary == artist.len()
}

pub(crate) fn filter_mpris_bus_names<I, S>(names: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut candidates = Vec::with_capacity(MPRIS_CANDIDATE_LIMIT);
    for name in names {
        let name = name.as_ref();
        if !name.starts_with(MPRIS_BUS_PREFIX) {
            continue;
        }
        if candidates.len() == MPRIS_CANDIDATE_LIMIT {
            return Err(format!(
                "more than {MPRIS_CANDIDATE_LIMIT} MPRIS candidates"
            ));
        }
        candidates.push(name.to_owned());
    }
    Ok(candidates)
}

pub(crate) fn consider_player(selected: &mut Option<MediaSnapshot>, candidate: MediaSnapshot) {
    if candidate.bus_name.is_none() {
        return;
    }
    let should_replace = selected.as_ref().is_none_or(|current| {
        candidate.playback_status.priority() > current.playback_status.priority()
            || (candidate.playback_status.priority() == current.playback_status.priority()
                && candidate.bus_name < current.bus_name)
    });
    if should_replace {
        *selected = Some(candidate);
    }
}

#[cfg(test)]
pub(crate) fn select_player(players: &[MediaSnapshot]) -> Option<&MediaSnapshot> {
    players
        .iter()
        .filter(|player| player.bus_name.is_some())
        .min_by(|left, right| {
            right
                .playback_status
                .priority()
                .cmp(&left.playback_status.priority())
                .then_with(|| left.bus_name.cmp(&right.bus_name))
        })
}

pub(crate) fn command_method(
    current: &MediaSnapshot,
    command: &MediaCommand,
) -> Option<&'static str> {
    if current.bus_name.as_deref() != Some(command.bus_name())
        || !current.supports(command.action())
    {
        return None;
    }

    Some(match command.action() {
        MediaAction::Previous => "Previous",
        MediaAction::PlayPause => "PlayPause",
        MediaAction::Next => "Next",
    })
}

pub(crate) async fn discover_player(connection: &Connection) -> zbus::Result<MediaSnapshot> {
    // ListNames necessarily arrives as one zbus-owned transport allocation. From
    // this boundary onward, name copies and player queries are capped explicitly.
    let names = DBusProxy::new(connection).await?.list_names().await?;
    let names = filter_mpris_bus_names(names.iter().map(|name| name.as_str()))
        .map_err(zbus::Error::Failure)?;
    let mut selected = None;

    for bus_name in names {
        // One non-conforming player must not hide healthy players on the same bus.
        if let Ok(player) = read_player(connection, &bus_name).await {
            consider_player(&mut selected, player);
        }
    }

    Ok(selected.unwrap_or_default())
}

async fn read_player(connection: &Connection, bus_name: &str) -> zbus::Result<MediaSnapshot> {
    let proxy = Proxy::new(connection, bus_name, PLAYER_PATH, PLAYER_INTERFACE).await?;
    let playback_status = proxy.get_property::<String>("PlaybackStatus").await?;
    let Some(playback_status) = parse_playback_status(&playback_status) else {
        return Err(zbus::Error::Failure(
            "invalid MPRIS playback status".to_owned(),
        ));
    };
    let metadata = proxy
        .get_property::<HashMap<String, OwnedValue>>("Metadata")
        .await
        .map(|metadata| parse_metadata(&metadata))
        .unwrap_or_default();
    let can_control = proxy
        .get_property::<bool>("CanControl")
        .await
        .unwrap_or(false);
    let capabilities = if can_control {
        MediaCapabilities {
            can_go_previous: proxy
                .get_property::<bool>("CanGoPrevious")
                .await
                .unwrap_or(false),
            can_play: proxy.get_property::<bool>("CanPlay").await.unwrap_or(false),
            can_pause: proxy
                .get_property::<bool>("CanPause")
                .await
                .unwrap_or(false),
            can_go_next: proxy
                .get_property::<bool>("CanGoNext")
                .await
                .unwrap_or(false),
        }
    } else {
        MediaCapabilities::default()
    };

    Ok(MediaSnapshot {
        bus_name: Some(bus_name.to_owned()),
        title: metadata.title,
        artist: metadata.artist,
        playback_status,
        capabilities,
        error: None,
    })
}

pub(crate) async fn execute_command(
    connection: &Connection,
    current: &MediaSnapshot,
    command: &MediaCommand,
) -> zbus::Result<()> {
    let Some(method) = command_method(current, command) else {
        return Ok(());
    };
    let proxy = Proxy::new(
        connection,
        command.bus_name(),
        PLAYER_PATH,
        PLAYER_INTERFACE,
    )
    .await?;
    proxy.call::<_, _, ()>(method, &()).await
}
