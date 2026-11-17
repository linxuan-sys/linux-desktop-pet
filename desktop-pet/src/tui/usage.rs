//! CPU is all process threads as a fraction of the entire machine's CPU time.
use crate::control;
use std::collections::BTreeMap;

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
    previous: BTreeMap<&'static str, (Snapshot, u64)>,
    page_size: u64,
    pub current: BTreeMap<&'static str, Usage>,
}
impl Monitor {
    pub fn new() -> Self {
        Self {
            previous: BTreeMap::new(),
            current: BTreeMap::new(),
            page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64,
        }
    }
    pub fn sample(&mut self) {
        let total = std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|text| machine_ticks(&text));
        for pet in crate::character::PETS {
            let pid = control::read_pid_for(pet);
            let snapshot = pid.and_then(|pid| {
                std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|text| parse_stat(pid, &text))
            });
            let usage = if let Some(snapshot) = snapshot {
                let cpu = total.and_then(|total| {
                    self.previous
                        .get(pet)
                        .and_then(|(old, old_total)| cpu_percent(*old, snapshot, *old_total, total))
                });
                if let Some(total) = total {
                    self.previous.insert(pet, (snapshot, total));
                } else {
                    self.previous.remove(pet);
                }
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
fn machine_ticks(text: &str) -> Option<u64> {
    let mut fields = text.lines().next()?.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    // guest and guest_nice are already included in user and nice, respectively.
    let values: Vec<_> = fields
        .take(8)
        .map(str::parse::<u64>)
        .collect::<Result<_, _>>()
        .ok()?;
    if values.len() < 4 {
        return None;
    }
    values.into_iter().try_fold(0u64, u64::checked_add)
}

fn cpu_percent(old: Snapshot, new: Snapshot, old_total: u64, total: u64) -> Option<f64> {
    if old.pid != new.pid || old.started != new.started {
        return None;
    }
    let elapsed = total.checked_sub(old_total)?;
    if elapsed == 0 {
        return None;
    }
    Some(new.ticks.checked_sub(old.ticks)? as f64 / elapsed as f64 * 100.0)
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
        assert_eq!(cpu_percent(a, b, 1000, 2600), Some(9.375));
        b.ticks = a.ticks + 100;
        assert_eq!(cpu_percent(a, b, 1000, 2600), Some(6.25));
        assert_eq!(cpu_percent(a, b, 1000, 1100), Some(100.0));
        assert_eq!(cpu_percent(a, b, 1000, 1000), None);
        assert_eq!(cpu_percent(a, b, 1000, 900), None);
        b.started += 1;
        assert_eq!(cpu_percent(a, b, 1000, 2600), None);
        fields[0] = "Z";
        assert!(parse_stat(42, &format!("42 (pet) {}", fields.join(" "))).is_none());
    }
    #[test]
    fn samples_the_real_process_rss() {
        let pid = std::process::id();
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        assert!(parse_stat(pid, &stat).unwrap().rss_pages > 0);
        assert!(machine_ticks(&std::fs::read_to_string("/proc/stat").unwrap()).unwrap() > 0);
    }
    #[test]
    fn machine_clock_includes_idle_and_counts_guest_time_once() {
        assert_eq!(
            machine_ticks("cpu 100 20 30 400 5 6 7 8 50 10\ncpu0 1 2 3 4"),
            Some(576)
        );
        assert_eq!(machine_ticks("cpu 1 2 3 4"), Some(10));
        assert_eq!(machine_ticks("cpu0 1 2 3 4"), None);
        assert_eq!(machine_ticks("cpu 1 invalid 3 4"), None);
    }
}
