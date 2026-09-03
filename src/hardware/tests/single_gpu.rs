use super::*;

#[test]
fn test_display_manager_service_names() {
    assert_eq!(DisplayManager::Gdm.service_name(), "gdm");
    assert_eq!(DisplayManager::Sddm.service_name(), "sddm");
    assert_eq!(DisplayManager::Lightdm.service_name(), "lightdm");
}

#[test]
fn test_gpu_driver_modules() {
    assert_eq!(GpuDriver::Nvidia.module_name(), "nvidia");
    assert!(!GpuDriver::Nvidia.dependent_modules().is_empty());
}

#[test]
fn gpu_rom_round_trips_through_config_file() {
    use crate::hardware::PciDevice;

    let dir = std::env::temp_dir().join(format!("vmcurator_sgpu_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let gpu = PciDevice {
        address: "0000:e4:00.0".to_string(),
        vendor_id: 0x1002,
        device_id: 0x1681,
        vendor_name: "AMD/ATI".to_string(),
        device_name: "Radeon 680M".to_string(),
        class_code: 0x030000,
        driver: Some("amdgpu".to_string()),
        iommu_group: Some(10),
        is_boot_vga: true,
        subsystem_vendor_id: 0,
        subsystem_device_id: 0,
    };
    let config = SingleGpuConfig {
        gpu,
        audio: None,
        iommu_group_devices: Vec::new(),
        original_driver: GpuDriver::Amdgpu,
        display_manager: DisplayManager::Gdm,
        gpu_rom: Some("/home/user/vbios.rom".to_string()),
        hide_kvm: true,
    };

    // A set ROM round-trips.
    save_config(&dir, &config).unwrap();
    let loaded = load_config(&dir).expect("config should load");
    assert_eq!(loaded.gpu_rom.as_deref(), Some("/home/user/vbios.rom"));
    assert_eq!(loaded.gpu.address, "0000:e4:00.0");

    // An unset ROM round-trips as None (key omitted).
    let config_none = SingleGpuConfig {
        gpu_rom: None,
        ..config
    };
    save_config(&dir, &config_none).unwrap();
    let loaded_none = load_config(&dir).expect("config should load");
    assert_eq!(loaded_none.gpu_rom, None);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Issue #71: hide_kvm must round-trip through the config file — in particular
/// an explicit `false` must survive, since the AMD/NVIDIA default is `true`.
#[test]
fn hide_kvm_round_trips_through_config_file() {
    use crate::hardware::PciDevice;

    let dir = std::env::temp_dir().join(format!(
        "vmcurator_sgpu_hidekvm_test_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let gpu = PciDevice {
        address: "0000:03:00.0".to_string(),
        vendor_id: 0x1002,
        device_id: 0x7550,
        vendor_name: "AMD/ATI".to_string(),
        device_name: "Radeon RX 9070 XT".to_string(),
        class_code: 0x030000,
        driver: Some("amdgpu".to_string()),
        iommu_group: Some(12),
        is_boot_vga: true,
        subsystem_vendor_id: 0,
        subsystem_device_id: 0,
    };
    let config = SingleGpuConfig {
        gpu,
        audio: None,
        iommu_group_devices: Vec::new(),
        original_driver: GpuDriver::Amdgpu,
        display_manager: DisplayManager::Sddm,
        gpu_rom: None,
        hide_kvm: false,
    };

    save_config(&dir, &config).unwrap();
    let loaded = load_config(&dir).expect("config should load");
    assert!(!loaded.hide_kvm, "explicit hide_kvm=false must round-trip");

    let config_on = SingleGpuConfig {
        hide_kvm: true,
        ..config
    };
    save_config(&dir, &config_on).unwrap();
    let loaded_on = load_config(&dir).expect("config should load");
    assert!(loaded_on.hide_kvm);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Configs written before #71 have no hide_kvm key; loading one must fall back
/// to the vendor default (on for AMD/NVIDIA GPUs).
#[test]
fn hide_kvm_defaults_on_when_missing_from_config_file() {
    let dir = std::env::temp_dir().join(format!(
        "vmcurator_sgpu_hidekvm_compat_test_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // A pre-#71 config file: [settings] has no hide_kvm key.
    std::fs::write(
        dir.join("single-gpu-config.toml"),
        r#"[gpu]
address = "0000:03:00.0"
vendor_id = "0x1002"
device_id = "0x7550"
vendor_name = "AMD/ATI"
device_name = "Radeon RX 9070 XT"
class_code = "0x030000"
driver = "amdgpu"
is_boot_vga = true

[settings]
original_driver = "amdgpu"
display_manager = "sddm"
"#,
    )
    .unwrap();

    let loaded = load_config(&dir).expect("config should load");
    assert!(
        loaded.hide_kvm,
        "AMD GPU without the key must default to on"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
