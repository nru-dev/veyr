use veyr::offsets::advanced_combat::hooks::Direct3d9Targets;

#[test]
fn d3d9_targets_require_two_live_method_entries() {
    assert!(Direct3d9Targets {
        end_scene: 0x1234_0000,
        reset: 0x1234_1000,
    }
    .is_valid());
    assert!(!Direct3d9Targets {
        end_scene: 0,
        reset: 0x1234_1000,
    }
    .is_valid());
    assert!(!Direct3d9Targets {
        end_scene: 0x1234_0000,
        reset: 0x1234_0000,
    }
    .is_valid());
}
