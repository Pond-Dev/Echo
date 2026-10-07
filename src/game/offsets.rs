//! Resolve addresses from the running game at attachment time.

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug)]
pub struct Offsets {
    pub view_angles: usize,
    pub local_player_pawn: usize,
    pub entity_system: usize,
    pub health: usize,
    pub team: usize,
    pub scene_node: usize,
    pub view_offset: usize,
    pub weapon_services: usize,
    pub active_handle: usize,
    pub item_definition: usize,
    pub pawn_handle: usize,
    pub bone_array: usize,
    pub origin: usize,
    pub entity: super::entities::Layout,
}

pub mod scene_node {
    pub const BONE_STRIDE: usize = 0x20;
    // Standard player rig; custom rigs need their own bone mapping.
    pub const HEAD: usize = 7;
    pub const HEAD_CAPSULE: [[f32; 3]; 2] = [[-1.0, 1.8, 0.0], [3.5, 0.2, 0.0]];
}

#[derive(Deserialize)]
struct TargetLayout {
    entity: super::entities::Layout,
    skeleton: SkeletonLayout,
}

#[derive(Deserialize)]
struct SkeletonLayout {
    bone_array_offset: usize,
    transform_stride: usize,
    translation_offset: usize,
    quaternion_offset: usize,
    scale_offset: usize,
}

impl Offsets {
    pub fn resolve(module_size: u32) -> Result<Self> {
        let dump = cs2_dumper::dump("cs2.exe").context("scanning the running game")?;
        Self::parse(&dump, module_size)
    }

    fn parse(dump: &cs2_dumper::DumpDocuments, module_size: u32) -> Result<Self> {
        let globals: Value = serde_json::from_slice(&dump.offsets_json)?;
        let schemas: Value = serde_json::from_slice(&dump.schemas_json)?;
        let layout: TargetLayout = serde_json::from_slice(&dump.target_layout_json)?;
        let global = |name: &str, width: usize| -> Result<usize> {
            let value = globals["client.dll"][name]
                .as_u64()
                .with_context(|| format!("missing client.dll.{name}"))?;
            ensure!(
                value > 0
                    && value
                        .checked_add(width as u64)
                        .is_some_and(|end| end <= u64::from(module_size)),
                "client.dll.{name} is outside the module"
            );
            Ok(usize::try_from(value)?)
        };
        let field = |class: &str, name: &str, width: usize| -> Result<usize> {
            let class_schema = &schemas["client.dll"]["classes"][class];
            let value = class_schema["fields"][name]
                .as_u64()
                .with_context(|| format!("missing {class}.{name}"))?;
            let size = class_schema["size"]
                .as_u64()
                .with_context(|| format!("missing {class} size"))?;
            ensure!(
                value
                    .checked_add(width as u64)
                    .is_some_and(|end| end <= size),
                "{class}.{name} is outside its class"
            );
            Ok(usize::try_from(value)?)
        };
        layout.entity.validate()?;
        // The transform decoder consumes this exact layout. Stop if it changes.
        ensure!(
            layout.skeleton.transform_stride == scene_node::BONE_STRIDE
                && layout.skeleton.translation_offset == 0
                && layout.skeleton.quaternion_offset == 0x10
                && layout.skeleton.scale_offset == 0x0c,
            "unsupported bone transform layout"
        );
        let model_state = field("CSkeletonInstance", "m_modelState", 1)?;
        let bone_array = model_state
            .checked_add(layout.skeleton.bone_array_offset)
            .context("bone array offset overflow")?;
        let skeleton_size = schemas["client.dll"]["classes"]["CSkeletonInstance"]["size"]
            .as_u64()
            .context("missing skeleton size")?;
        ensure!(
            bone_array
                .checked_add(8)
                .is_some_and(|end| end as u64 <= skeleton_size),
            "bone array is outside the skeleton"
        );
        let attribute_manager = field("C_EconEntity", "m_AttributeManager", 1)?;
        let item = field("C_AttributeContainer", "m_Item", 1)?;
        let definition = field("C_EconItemView", "m_iItemDefinitionIndex", 2)?;
        let item_definition = attribute_manager
            .checked_add(item)
            .and_then(|offset| offset.checked_add(definition))
            .context("item definition offset overflow")?;
        Ok(Self {
            view_angles: global("dwViewAngles", 8)?,
            local_player_pawn: global("dwLocalPlayerPawn", 8)?,
            entity_system: global("dwEntityList", 8)?,
            health: field("C_BaseEntity", "m_iHealth", 4)?,
            team: field("C_BaseEntity", "m_iTeamNum", 1)?,
            scene_node: field("C_BaseEntity", "m_pGameSceneNode", 8)?,
            view_offset: field("C_BaseModelEntity", "m_vecViewOffset", 12)?,
            weapon_services: field("C_BasePlayerPawn", "m_pWeaponServices", 8)?,
            active_handle: field("CPlayer_WeaponServices", "m_hActiveWeapon", 4)?,
            item_definition,
            pawn_handle: field("CCSPlayerController", "m_hPlayerPawn", 4)?,
            bone_array,
            origin: field("CGameSceneNode", "m_vecAbsOrigin", 12)?,
            entity: layout.entity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> cs2_dumper::DumpDocuments {
        let mut classes = json!({});
        for (class, field, offset) in [
            ("C_BaseEntity", "m_iHealth", 0x34c),
            ("C_BaseEntity", "m_iTeamNum", 0x3e7),
            ("C_BaseEntity", "m_pGameSceneNode", 0x330),
            ("C_BaseModelEntity", "m_vecViewOffset", 0xf60),
            ("C_BasePlayerPawn", "m_pWeaponServices", 0x12f0),
            ("CPlayer_WeaponServices", "m_hActiveWeapon", 0x60),
            ("C_EconEntity", "m_AttributeManager", 0x1148),
            ("C_AttributeContainer", "m_Item", 0x50),
            ("C_EconItemView", "m_iItemDefinitionIndex", 0x1ba),
            ("CCSPlayerController", "m_hPlayerPawn", 0x92c),
            ("CSkeletonInstance", "m_modelState", 0x140),
            ("CGameSceneNode", "m_vecAbsOrigin", 0xc8),
        ] {
            if classes.get(class).is_none() {
                classes[class] = json!({"size": 0x2000, "fields": {}});
            }
            classes[class]["fields"][field] = json!(offset);
        }
        cs2_dumper::DumpDocuments {
            offsets_json: serde_json::to_vec(&json!({"client.dll": {
                "dwViewAngles": 0x1000, "dwLocalPlayerPawn": 0x2000,
                "dwEntityList": 0x3000,
            }}))
            .unwrap(),
            schemas_json: serde_json::to_vec(&json!({"client.dll": {"classes": classes}})).unwrap(),
            target_layout_json: serde_json::to_vec(&json!({
                "entity": {"chunk_pointer_base": 0x10, "chunk_pointer_stride": 8,
                    "entry_stride": 0x70, "chunk_size": 512, "chunk_count": 64,
                    "handle_index_mask": 0x7fff},
                "skeleton": {"bone_array_offset": 0x80, "transform_stride": 0x20,
                    "translation_offset": 0, "quaternion_offset": 0x10, "scale_offset": 0xc},
            }))
            .unwrap(),
        }
    }

    #[test]
    fn parses_fresh_values_and_rejects_incomplete_or_unsupported_dumps() {
        let mut dump = fixture();
        let offsets = Offsets::parse(&dump, 0x10000).unwrap();
        assert_eq!(offsets.item_definition, 0x1148 + 0x50 + 0x1ba);
        assert_eq!(offsets.bone_array, 0x1c0);
        assert!(Offsets::parse(&dump, 0x3004).is_err());

        // A new dump changes the values actually used, including entity strides.
        let mut globals: Value = serde_json::from_slice(&dump.offsets_json).unwrap();
        globals["client.dll"]["dwViewAngles"] = json!(0x4000);
        dump.offsets_json = serde_json::to_vec(&globals).unwrap();
        let mut layout: Value = serde_json::from_slice(&dump.target_layout_json).unwrap();
        layout["entity"]["entry_stride"] = json!(0x78);
        layout["skeleton"]["bone_array_offset"] = json!(0x90);
        dump.target_layout_json = serde_json::to_vec(&layout).unwrap();
        let fresh = Offsets::parse(&dump, 0x10000).unwrap();
        assert_eq!(fresh.view_angles, 0x4000);
        assert_eq!(fresh.entity.entry_offset(2), 0xf0);
        assert_eq!(fresh.bone_array, 0x1d0);
        layout["skeleton"]["transform_stride"] = json!(0x40);
        dump.target_layout_json = serde_json::to_vec(&layout).unwrap();
        assert!(Offsets::parse(&dump, 0x10000).is_err());

        let mut missing = fixture();
        let mut schemas: Value = serde_json::from_slice(&missing.schemas_json).unwrap();
        schemas["client.dll"]["classes"]["C_BaseEntity"]["fields"]
            .as_object_mut()
            .unwrap()
            .remove("m_iHealth");
        missing.schemas_json = serde_json::to_vec(&schemas).unwrap();
        assert!(
            format!("{:#}", Offsets::parse(&missing, 0x10000).unwrap_err()).contains("m_iHealth")
        );
    }

    #[test]
    #[ignore = "requires CS2 running and permission to read its memory"]
    fn live_offsets() {
        let game = crate::game::Game::attach()
            .unwrap()
            .expect("client.dll loaded");
        println!("resolved offsets: {:?}", game.offsets());
        assert!(game.client_looks_like_a_module().unwrap());
        let view = game.view_angles().unwrap();
        assert!(view.plausible(), "invalid view angles: {view:?}");
        let local = game.local_player().unwrap();
        assert!(local.is_none_or(|player| player.plausible()));
        let players = game.players().unwrap();
        assert!(players.iter().all(|player| player.plausible()));
        println!("view={view:?} local={local:?} players={}", players.len());
    }
}
