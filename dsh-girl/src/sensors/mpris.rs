// music via MPRIS on the user bus.
// "Playing wins": any player reporting PlaybackStatus=Playing is the one the
// pet listens to.

use zbus::blocking::Connection;

pub struct Music {
    pub playing: bool,
}

pub fn parse(playback_status: Option<&str>) -> Music {
    Music {
        playing: playback_status == Some("Playing"),
    }
}

/// Music state across all MPRIS players on the user bus.
pub fn read() -> Music {
    let Ok(conn) = Connection::session() else {
        return Music { playing: false };
    };
    let Ok(names) = zbus::blocking::fdo::DBusProxy::new(&conn)
        .and_then(|p| p.list_names().map_err(zbus::Error::from))
    else {
        return Music { playing: false };
    };

    let mut fallback: Option<Music> = None;
    for name in names.iter().map(|n| n.as_str()).filter(|n| {
        n.starts_with("org.mpris.MediaPlayer2.") && !n.ends_with(".playerctld")
    }) {
        let Ok(proxy) = zbus::blocking::Proxy::new(&conn, name, "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player") else {
            continue;
        };
        let Ok(status): Result<String, _> = proxy.get_property("PlaybackStatus") else {
            continue;
        };
        let music = parse(Some(&status));
        if music.playing {
            return music;
        }
        fallback = fallback.or(Some(music));
    }
    fallback.unwrap_or(Music { playing: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playing_is_playing() {
        assert!(parse(Some("Playing")).playing);
    }

    #[test]
    fn paused_is_not_playing() {
        assert!(!parse(Some("Paused")).playing);
    }

    #[test]
    fn nothing_playing() {
        assert!(!parse(None).playing);
    }
}
