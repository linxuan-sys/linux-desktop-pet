// Platform-neutral state shape shared by the sensor layer.

#[derive(Debug, Clone)]
pub struct DerivedState {
    pub mode: String,
    pub spotify_playing: bool,
    pub hour: u32,
    pub battery_pct: u32,
    pub is_charging: bool,
}
