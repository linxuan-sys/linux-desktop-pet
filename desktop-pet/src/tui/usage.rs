//! One /proc snapshot per second; CPU is the sum of all process threads.
use crate::control;
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
struct Snapshot {
    pid: u32,
    started: u64,
    ticks: u64,
    rss_pages: u64,
}

fn parse_stat(pid: u32, stat: &str) -> Option<Snapshot> {
    // comm may contain spaces and parentheses. Fields after its final ')' start at state (#3).
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    if matches!(*fields.first()?, "Z" | "X") {
        return None;
    }
    Some(Snapshot {
        pid,
        started: fields.get(19)?.parse().ok()?,
        ticks: fields
            .get(11)?
            .parse::<u64>()
            .ok()?
            .checked_add(fields.get(12)?.parse().ok()?)?,
        rss_pages: fields.get(21)?.parse::<i64>().ok()?.max(0) as u64,
    })
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Usage {
    pub pid: Option<u32>,
    pub cpu: Option<f64>,
    pub rss: Option<u64>,
}

pub struct Monitor {
    previous: BTreeMap<&'static str, (Snapshot, Instant)>,
    hz: f64,
    page_size: u64,
    pub current: BTreeMap<&'static str, Usage>,
}
impl Monitor {
    pub fn new() -> Self {
        Self {
            previous: BTreeMap::new(),
            current: BTreeMap::new(),
            hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64,
            page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64,
        }
    }
    pub fn sample(&mut self) {
        for pet in crate::character::PETS {
            let pid = control::read_pid_for(pet);
            let snapshot = pid.and_then(|pid| {
                std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|text| parse_stat(pid, &text))
            });
            let now = Instant::now();
            let usage = if let Some(snapshot) = snapshot {
                let cpu = self.previous.get(pet).and_then(|(old, time)| {
                    cpu_percent(
                        *old,
                        snapshot,
                        now.duration_since(*time).as_secs_f64(),
                        self.hz,
                    )
                });
                self.previous.insert(pet, (snapshot, now));
                Usage {
                    pid,
                    cpu,
                    rss: Some(snapshot.rss_pages.saturating_mul(self.page_size)),
                }
            } else {
                self.previous.remove(pet);
                Usage {
                    pid,
                    ..Usage::default()
                }
            };
            self.current.insert(pet, usage);
        }
    }
}
fn cpu_percent(old: Snapshot, new: Snapshot, seconds: f64, hz: f64) -> Option<f64> {
    if old.pid != new.pid || old.started != new.started || seconds <= 0.0 {
        return None;
    }
    Some(new.ticks.checked_sub(old.ticks)? as f64 / hz / seconds * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_names_with_spaces_and_parentheses_and_all_thread_cpu() {
        let mut fields = vec!["0"; 22];
        fields[0] = "S";
        fields[11] = "180";
        fields[12] = "20";
        fields[19] = "1000";
        fields[21] = "512";
        let stat = format!("42 (pet (codex)) {}", fields.join(" "));
        let a = parse_stat(42, &stat).unwrap();
        assert_eq!(a.ticks, 200);
        assert_eq!(a.rss_pages, 512);
        let mut b = a;
        b.ticks += 150;
        assert_eq!(cpu_percent(a, b, 1.0, 100.0), Some(150.0));
        b.started += 1;
        assert_eq!(cpu_percent(a, b, 1.0, 100.0), None);
        fields[0] = "Z";
        assert!(parse_stat(42, &format!("42 (pet) {}", fields.join(" "))).is_none());
    }
    #[test]
    fn samples_the_real_process_rss() {
        let pid = std::process::id();
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        assert!(parse_stat(pid, &stat).unwrap().rss_pages > 0);
    }
}
