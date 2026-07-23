use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

const MAX_HWMON_DEVICES: usize = 64;
const MAX_DEVICE_ENTRIES: usize = 256;
const MAX_SENSOR_FILE_BYTES: u64 = 128;
const MIN_TEMPERATURE_MILLICELSIUS: i64 = -20_000;
const MAX_TEMPERATURE_MILLICELSIUS: i64 = 150_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TemperatureSnapshot {
    pub cpu_millicelsius: Option<i64>,
    pub gpu_millicelsius: Option<i64>,
}

#[derive(Clone, Copy, Debug)]
struct PreferredTemperature {
    priority: u8,
    value: Option<i64>,
}

pub fn scan_temperatures(class_root: &Path, devices_root: &Path) -> TemperatureSnapshot {
    scan_temperatures_inner(class_root, devices_root).unwrap_or_default()
}

fn scan_temperatures_inner(
    class_root: &Path,
    devices_root: &Path,
) -> io::Result<TemperatureSnapshot> {
    let class_root = fs::canonicalize(class_root)?;
    let devices_root = fs::canonicalize(devices_root)?;
    let Some(class_entries) = bounded_entries(&class_root, MAX_HWMON_DEVICES)? else {
        return Ok(TemperatureSnapshot::default());
    };
    let mut cpu = None;
    let mut gpu = None;

    for class_entry in class_entries {
        let Ok(device) = fs::canonicalize(class_entry) else {
            continue;
        };
        if device == devices_root || !device.starts_with(&devices_root) || !device.is_dir() {
            continue;
        }
        let Some(driver) = read_bounded_file(&device, "name") else {
            continue;
        };
        let Some(device_entries) = bounded_entries(&device, MAX_DEVICE_ENTRIES)? else {
            continue;
        };
        for entry in device_entries {
            let Some(index) = sensor_input_index(&entry) else {
                continue;
            };
            let Some(label) = read_bounded_file(&device, &format!("temp{index}_label")) else {
                continue;
            };
            let Some(value) = read_bounded_file(&device, &format!("temp{index}_input"))
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|value| {
                    (MIN_TEMPERATURE_MILLICELSIUS..=MAX_TEMPERATURE_MILLICELSIUS).contains(value)
                })
            else {
                continue;
            };

            if let Some(priority) = cpu_priority(&driver, &label) {
                prefer(&mut cpu, priority, value);
            }
            if let Some(priority) = gpu_priority(&driver, &label) {
                prefer(&mut gpu, priority, value);
            }
        }
    }

    Ok(TemperatureSnapshot {
        cpu_millicelsius: cpu.and_then(|candidate| candidate.value),
        gpu_millicelsius: gpu.and_then(|candidate| candidate.value),
    })
}

fn bounded_entries(root: &Path, limit: usize) -> io::Result<Option<Vec<PathBuf>>> {
    let mut entries = Vec::with_capacity(limit);
    for entry in fs::read_dir(root)?.take(limit + 1) {
        let entry = entry?;
        if entries.len() == limit {
            return Ok(None);
        }
        entries.push(entry.path());
    }
    entries.sort_unstable();
    Ok(Some(entries))
}

fn read_bounded_file(device: &Path, name: &str) -> Option<String> {
    let path = fs::canonicalize(device.join(name)).ok()?;
    if path == device || !path.starts_with(device) || !path.is_file() {
        return None;
    }
    let mut contents = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_SENSOR_FILE_BYTES + 1)
        .read_to_end(&mut contents)
        .ok()?;
    if contents.len() > usize::try_from(MAX_SENSOR_FILE_BYTES).ok()? {
        return None;
    }
    let contents = String::from_utf8(contents).ok()?;
    Some(contents.trim().to_owned())
}

fn sensor_input_index(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let index = name.strip_prefix("temp")?.strip_suffix("_input")?;
    (!index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| index.parse().ok())?
}

fn cpu_priority(driver: &str, label: &str) -> Option<u8> {
    match (driver, label) {
        ("k10temp" | "zenpower", "Tctl") => Some(0),
        ("k10temp" | "zenpower", "Tdie") => Some(1),
        ("coretemp", label) if label.starts_with("Package id ") => Some(0),
        _ => None,
    }
}

fn gpu_priority(driver: &str, label: &str) -> Option<u8> {
    match (driver, label) {
        ("amdgpu", "edge") => Some(0),
        ("amdgpu", "junction") => Some(1),
        _ => None,
    }
}

fn prefer(selection: &mut Option<PreferredTemperature>, priority: u8, value: i64) {
    match selection {
        Some(current) if current.priority < priority => {}
        Some(current) if current.priority == priority => current.value = None,
        _ => {
            *selection = Some(PreferredTemperature {
                priority,
                value: Some(value),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink, path::Path};

    use tempfile::TempDir;

    use super::{TemperatureSnapshot, scan_temperatures};

    struct Fixture {
        _temp: TempDir,
        class_root: std::path::PathBuf,
        devices_root: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let sys_root = temp.path().join("sys");
            let class_root = sys_root.join("class/hwmon");
            let devices_root = sys_root.join("devices");
            fs::create_dir_all(&class_root).unwrap();
            fs::create_dir_all(&devices_root).unwrap();
            Self {
                _temp: temp,
                class_root,
                devices_root,
            }
        }

        fn device(&self, index: usize, name: &str) -> std::path::PathBuf {
            let device = self.devices_root.join(format!("platform/device-{index}"));
            fs::create_dir_all(&device).unwrap();
            fs::write(device.join("name"), name).unwrap();
            symlink(&device, self.class_root.join(format!("hwmon{index}"))).unwrap();
            device
        }

        fn scan(&self) -> TemperatureSnapshot {
            scan_temperatures(&self.class_root, &self.devices_root)
        }
    }

    fn sensor(device: &Path, index: usize, label: &str, value: &str) {
        fs::write(device.join(format!("temp{index}_label")), label).unwrap();
        fs::write(device.join(format!("temp{index}_input")), value).unwrap();
    }

    #[test]
    fn exact_driver_and_label_preferences_select_cpu_and_gpu_temperatures() {
        let fixture = Fixture::new();
        let cpu = fixture.device(0, "k10temp\n");
        sensor(&cpu, 1, "Tdie\n", "55000\n");
        sensor(&cpu, 2, "Tctl\n", "65000\n");
        let gpu = fixture.device(1, "amdgpu\n");
        sensor(&gpu, 1, "junction\n", "90000\n");
        sensor(&gpu, 2, "edge\n", "70000\n");

        assert_eq!(
            fixture.scan(),
            TemperatureSnapshot {
                cpu_millicelsius: Some(65_000),
                gpu_millicelsius: Some(70_000),
            }
        );
    }

    #[test]
    fn out_of_range_temperatures_are_unavailable() {
        let fixture = Fixture::new();
        let cpu = fixture.device(0, "k10temp");
        sensor(&cpu, 1, "Tctl", "-20001");
        let gpu = fixture.device(1, "amdgpu");
        sensor(&gpu, 1, "edge", "150001");

        assert_eq!(fixture.scan(), TemperatureSnapshot::default());
    }

    #[test]
    fn inclusive_temperature_range_boundaries_are_accepted() {
        let fixture = Fixture::new();
        let cpu = fixture.device(0, "k10temp");
        sensor(&cpu, 1, "Tctl", "-20000");
        let gpu = fixture.device(1, "amdgpu");
        sensor(&gpu, 1, "edge", "150000");

        assert_eq!(
            fixture.scan(),
            TemperatureSnapshot {
                cpu_millicelsius: Some(-20_000),
                gpu_millicelsius: Some(150_000),
            }
        );
    }

    #[test]
    fn class_symlinks_outside_the_injected_device_tree_are_ignored() {
        let fixture = Fixture::new();
        let escaped = fixture._temp.path().join("escaped-device");
        fs::create_dir(&escaped).unwrap();
        fs::write(escaped.join("name"), "k10temp").unwrap();
        sensor(&escaped, 1, "Tctl", "65000");
        symlink(&escaped, fixture.class_root.join("hwmon0")).unwrap();

        assert_eq!(fixture.scan(), TemperatureSnapshot::default());
    }

    #[test]
    fn sensor_files_escaping_the_canonical_device_are_ignored() {
        let fixture = Fixture::new();
        let cpu = fixture.device(0, "k10temp");
        let escaped = fixture._temp.path().join("escaped-input");
        fs::write(&escaped, "65000").unwrap();
        fs::write(cpu.join("temp1_label"), "Tctl").unwrap();
        symlink(&escaped, cpu.join("temp1_input")).unwrap();

        assert_eq!(fixture.scan(), TemperatureSnapshot::default());
    }

    #[test]
    fn oversized_sensor_files_are_ignored() {
        let fixture = Fixture::new();
        let cpu = fixture.device(0, "k10temp");
        fs::write(cpu.join("temp1_label"), "Tctl").unwrap();
        fs::write(cpu.join("temp1_input"), "6".repeat(1024)).unwrap();

        assert_eq!(fixture.scan(), TemperatureSnapshot::default());
    }
}
