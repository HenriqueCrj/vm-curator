//! Physical block device enumeration for whole-disk passthrough.
//!
//! Enumerates whole disks from /sys/block, resolves stable /dev/disk/by-id
//! paths, and flags devices that are unsafe to pass to a guest (host system
//! disk, mounted partitions, swap, LVM/LUKS/md members).

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Bus a block device is attached through
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockBus {
    Nvme,
    Sata,
    Usb,
    Virtio,
    Other(String),
}

impl BlockBus {
    pub fn label(&self) -> &str {
        match self {
            Self::Nvme => "NVMe",
            Self::Sata => "SATA",
            Self::Usb => "USB",
            Self::Virtio => "VirtIO",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Why a device must not be passed through to a guest
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExclusionReason {
    /// A partition is mounted at /, /boot, or /boot/efi
    HostSystemDisk,
    /// Partitions mounted elsewhere (mountpoints listed)
    Mounted(Vec<String>),
    /// A partition is in use as swap
    SwapMember,
    /// Disk or a partition is held by device-mapper/md (LVM, LUKS, RAID)
    HeldByDeviceMapper,
}

impl ExclusionReason {
    pub fn label(&self) -> String {
        match self {
            Self::HostSystemDisk => "host system disk".to_string(),
            Self::Mounted(points) => format!("mounted at {}", points.join(", ")),
            Self::SwapMember => "in use as swap".to_string(),
            Self::HeldByDeviceMapper => "LVM/LUKS/RAID member".to_string(),
        }
    }
}

/// A whole physical disk discovered on the host
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDevice {
    /// Kernel name, e.g. "nvme0n1"
    pub name: String,
    /// /dev/<name>
    pub dev_path: PathBuf,
    /// Stable /dev/disk/by-id symlink, preferred for launch scripts
    pub by_id_path: Option<PathBuf>,
    pub model: String,
    pub vendor: Option<String>,
    pub size_bytes: u64,
    pub removable: bool,
    pub rotational: bool,
    pub bus: BlockBus,
    /// Set when the device must not be passed through
    pub exclusion: Option<ExclusionReason>,
}

impl BlockDevice {
    /// Path to reference in generated launch scripts (stable by-id if known)
    pub fn launch_path(&self) -> &Path {
        self.by_id_path.as_deref().unwrap_or(&self.dev_path)
    }

    pub fn size_display(&self) -> String {
        format_size(self.size_bytes)
    }

    pub fn is_selectable(&self) -> bool {
        self.exclusion.is_none()
    }
}

pub fn format_size(bytes: u64) -> String {
    const GB: f64 = 1_000_000_000.0;
    const TB: f64 = 1_000_000_000_000.0;
    let b = bytes as f64;
    if b >= TB {
        format!("{:.2} TB", b / TB)
    } else if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", b / 1_000_000.0)
    }
}

/// Enumerate physical disks suitable (or explicitly unsuitable) for passthrough
pub fn enumerate_block_devices() -> Result<Vec<BlockDevice>> {
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let swaps = fs::read_to_string("/proc/swaps").unwrap_or_default();
    enumerate_block_devices_at(
        Path::new("/sys/block"),
        Path::new("/dev/disk/by-id"),
        &mounts,
        &swaps,
    )
}

/// Testable core: enumerate from an arbitrary sysfs root and by-id directory,
/// with /proc/mounts and /proc/swaps contents passed in as strings.
pub fn enumerate_block_devices_at(
    sysfs_block: &Path,
    dev_by_id: &Path,
    mounts: &str,
    swaps: &str,
) -> Result<Vec<BlockDevice>> {
    let mut devices = Vec::new();
    if !sysfs_block.exists() {
        return Ok(devices);
    }

    let by_id_map = build_by_id_map(dev_by_id);

    for entry in fs::read_dir(sysfs_block)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        if is_virtual_device(&name) {
            continue;
        }

        let dev_dir = entry.path();
        let size_bytes = read_attr(&dev_dir, "size")
            .and_then(|s| s.parse::<u64>().ok())
            .map(|sectors| sectors * 512)
            .unwrap_or(0);
        if size_bytes == 0 {
            continue; // empty card readers etc.
        }

        let model = read_attr(&dev_dir, "device/model").unwrap_or_default();
        let vendor = read_attr(&dev_dir, "device/vendor").filter(|v| !v.is_empty());
        let removable = read_attr(&dev_dir, "removable").as_deref() == Some("1");
        let rotational = read_attr(&dev_dir, "queue/rotational").as_deref() == Some("1");
        let bus = detect_bus(&name, &dev_dir);

        let exclusion = detect_exclusion(&name, &dev_dir, mounts, swaps);

        devices.push(BlockDevice {
            dev_path: PathBuf::from(format!("/dev/{}", name)),
            by_id_path: by_id_map
                .iter()
                .find(|(_, target)| *target == name)
                .map(|(link, _)| link.clone()),
            name,
            model,
            vendor,
            size_bytes,
            removable,
            rotational,
            bus,
            exclusion,
        });
    }

    devices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(devices)
}

/// Kernel names that are never physical disks
fn is_virtual_device(name: &str) -> bool {
    ["loop", "zram", "ram", "fd", "sr", "dm-", "md"]
        .iter()
        .any(|p| {
            name.starts_with(p)
                && name[p.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| c.is_ascii_digit())
        })
}

fn read_attr(dir: &Path, attr: &str) -> Option<String> {
    fs::read_to_string(dir.join(attr))
        .ok()
        .map(|s| s.trim().to_string())
}

fn detect_bus(name: &str, dev_dir: &Path) -> BlockBus {
    if name.starts_with("nvme") {
        return BlockBus::Nvme;
    }
    if name.starts_with("vd") {
        return BlockBus::Virtio;
    }
    if name.starts_with("mmcblk") {
        return BlockBus::Other("SD/MMC".to_string());
    }
    // USB disks appear as sdX; detect via the resolved sysfs device path
    if let Ok(real) = fs::canonicalize(dev_dir) {
        if real.to_string_lossy().contains("/usb") {
            return BlockBus::Usb;
        }
    }
    BlockBus::Sata
}

/// Sorted list of (by-id symlink path, target kernel name), whole disks only
fn build_by_id_map(dev_by_id: &Path) -> Vec<(PathBuf, String)> {
    let mut map = Vec::new();
    let Ok(entries) = fs::read_dir(dev_by_id) else {
        return map;
    };
    for entry in entries.flatten() {
        let link = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        // Skip partition links (…-part1) — we pass through whole disks
        if partition_link(&file_name) {
            continue;
        }
        let Ok(target) = fs::read_link(&link) else {
            continue;
        };
        // Targets are relative like ../../nvme0n1
        let Some(kernel_name) = target.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        map.push((link, kernel_name));
    }
    // Prefer model-serial style names (ata-/nvme-/usb-/scsi-) over wwn-/eui
    // links: sort so preferred prefixes come first; the find() in the caller
    // picks the first match.
    map.sort_by_key(|(link, _)| {
        let n = link.file_name().map(|n| n.to_string_lossy().to_string());
        let n = n.unwrap_or_default();
        let preferred = ["ata-", "nvme-", "usb-", "scsi-"]
            .iter()
            .any(|p| n.starts_with(p));
        let generic = n.starts_with("wwn-") || n.starts_with("nvme-eui.");
        (generic, !preferred, n)
    });
    map
}

fn partition_link(name: &str) -> bool {
    if let Some(idx) = name.rfind("-part") {
        return name[idx + 5..].chars().all(|c| c.is_ascii_digit()) && name.len() > idx + 5;
    }
    false
}

/// True if `source` (e.g. /dev/sda1, /dev/nvme0n1p2) is the disk `name` or a
/// partition of it.
fn source_matches_disk(source: &str, name: &str) -> bool {
    let Some(dev) = source.strip_prefix("/dev/") else {
        return false;
    };
    if dev == name {
        return true;
    }
    let Some(rest) = dev.strip_prefix(name) else {
        return false;
    };
    // Partition suffix: "1" (sda1) or "p1" (nvme0n1p1)
    let rest = rest.strip_prefix('p').unwrap_or(rest);
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn detect_exclusion(
    name: &str,
    dev_dir: &Path,
    mounts: &str,
    swaps: &str,
) -> Option<ExclusionReason> {
    // 1. Mounted partitions (host system disk gets its own label)
    let mut mountpoints = Vec::new();
    let mut is_system = false;
    for line in mounts.lines() {
        let mut cols = line.split_whitespace();
        let (Some(source), Some(target)) = (cols.next(), cols.next()) else {
            continue;
        };
        if source_matches_disk(source, name) {
            if matches!(target, "/" | "/boot" | "/boot/efi") {
                is_system = true;
            }
            mountpoints.push(target.to_string());
        }
    }
    if is_system {
        return Some(ExclusionReason::HostSystemDisk);
    }
    if !mountpoints.is_empty() {
        return Some(ExclusionReason::Mounted(mountpoints));
    }

    // 2. Swap
    for line in swaps.lines().skip(1) {
        if let Some(source) = line.split_whitespace().next() {
            if source_matches_disk(source, name) {
                return Some(ExclusionReason::SwapMember);
            }
        }
    }

    // 3. Held by device-mapper / md (LVM, LUKS, RAID) — check holders/ on the
    // disk and on each partition. Covers dm-rooted hosts where /proc/mounts
    // only shows /dev/mapper/... paths.
    if has_holders(dev_dir) {
        return Some(ExclusionReason::HeldByDeviceMapper);
    }
    if let Ok(entries) = fs::read_dir(dev_dir) {
        for entry in entries.flatten() {
            let part_name = entry.file_name().to_string_lossy().to_string();
            if part_name.starts_with(name) && has_holders(&entry.path()) {
                return Some(ExclusionReason::HeldByDeviceMapper);
            }
        }
    }

    None
}

fn has_holders(dir: &Path) -> bool {
    fs::read_dir(dir.join("holders"))
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "tests/block.rs"]
mod tests;
