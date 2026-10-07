use anyhow::{Result, anyhow, bail, ensure};
use memflow::prelude::v1::*;
use pelite::pattern;
use pelite::pattern::{Atom, save_len};
use pelite::pe64::{Pe, PeView};
use serde::{Deserialize, Serialize};

use super::SchemaMap;

const ENTITY_RESOLVER_PATTERN: &[Atom] = pattern!(
    "33d2 83f9ff 74? 4c8b05???? 4d85c0 74? 83f9fe 74? 8bc1 25u4 448bc8 48c1e8u1 4d8b14u1 4d85d2 74? 4181e1u4 496bc1u1 4903c2 74? 3948u1 480f45c2 eb? 488bc2"
);
const ENTITY_CHUNK_BASE_PATTERN: &[Atom] =
    pattern!("488b3d???? 488d43u1 4533ff 48891d???? 4885db 490f44c7 488905???? 4885ff 74?");
const SKELETON_BONE_ARRAY_PATTERN: &[Atom] =
    pattern!("4c8ba3u4 418d45ff 4c63f8 85c0 0f88???? 6690");
const BONE_TRANSFORM_STRIDE_PATTERN: &[Atom] =
    pattern!("48635e? 4c8d45? 48c1e3u1 488d55? 4903dc f30f1053?");
const TRANSFORM_COMPONENT_PATTERN: &[Atom] = pattern!(
    "f30f1053u1 488d4bu1 f20f101b 0f16da 0f295d? e8???? 0f1000 0f1145? 0f104810 0f114d? 0f104020 0f1145? f30f1053u1"
);
const MODEL_NAME_PATTERN: &[Atom] = pattern!(
    "8b0d???? ba03000000 ff15???? 84c0 74? 498b87u4 488d3d???? 498b4f38 4885c0 488bdf 480f45d8 488b01 ff5070 4c8d05???? 48895c2420 ba03000000 488b88u4 4885c9 480f45f9 8b0d???? 4c8bcf ff15????"
);
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetLayout {
    pub entity: EntityLayout,
    pub skeleton: SkeletonLayout,
    pub model_name: ModelNameLayout,
    pub spotted: Option<SpottedLayout>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EntityLayout {
    pub chunk_pointer_base: u32,
    pub chunk_pointer_stride: u32,
    pub entry_stride: u32,
    /// Entry DWORD compared against the resolver's unmasked raw handle.
    pub handle_identity_offset: u32,
    pub chunk_size: u32,
    pub chunk_count: u32,
    pub handle_index_mask: u32,
    pub handle_serial_shift: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkeletonLayout {
    pub bone_array_offset: u32,
    pub transform_stride: u32,
    pub translation_offset: u32,
    pub quaternion_offset: u32,
    pub scale_offset: u32,
    pub pose_generation: Option<PoseGenerationLayout>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PoseGenerationLayout {
    pub offset: u32,
    pub width: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelNameLayout {
    pub symbol_data_offset: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpottedLayout {
    pub pawn_mask_offset: u32,
    pub word_width: u32,
    pub word_count: u32,
}

#[derive(Clone, Copy, Debug)]
struct LayoutEvidence {
    chunk_pointer_base: u8,
    chunk_pointer_sib: u8,
    entry_stride: u8,
    handle_identity_offset: u8,
    chunk_shift: u8,
    chunk_index_mask: u32,
    handle_index_mask: u32,
    skeleton_bone_array_offset: u32,
    transform_stride_shift: u8,
    translation_z_offset: u8,
    quaternion_offset: u8,
    scale_offset: u8,
    skeleton_model_name_offset: u32,
    second_skeleton_model_name_offset: u32,
}

pub fn layouts<P: Process + MemoryView>(
    process: &mut P,
    schemas: &SchemaMap,
) -> Result<TargetLayout> {
    let client_module = process.module_by_name("client.dll")?;
    let client_buf = process
        .read_raw(client_module.base, client_module.size as _)
        .data_part()?;
    let client_view = PeView::from_bytes(&client_buf)?;
    let layout_evidence = capture_layout_evidence(client_view)?;
    drop(client_buf);
    let model_state_offset = schema_field(schemas, "CSkeletonInstance", "m_modelState")?;
    let model_name_offset = schema_field(schemas, "CModelState", "m_ModelName")?;

    let mut layout =
        resolve_layout_evidence(layout_evidence, model_state_offset, model_name_offset)?;
    layout.spotted = match resolve_spotted_layout(schemas) {
        Ok(spotted) => Some(spotted),
        Err(error) => {
            log::warn!("optional spotted layout unavailable: {error:#}");
            None
        }
    };
    Ok(layout)
}

fn schema_field(schemas: &SchemaMap, class_name: &str, field_name: &str) -> Result<u32> {
    schema_field_definition(schemas, "client.dll", class_name, field_name).map(|(offset, _)| offset)
}

fn schema_field_definition<'a>(
    schemas: &'a SchemaMap,
    module_name: &str,
    class_name: &str,
    field_name: &str,
) -> Result<(u32, &'a str)> {
    let (classes, _) = schemas
        .get(module_name)
        .ok_or_else(|| anyhow!("target layout: missing {module_name} schema"))?;
    let class = classes
        .iter()
        .find(|class| class.name == class_name)
        .ok_or_else(|| anyhow!("target layout: {module_name} missing class {class_name}"))?;
    let field = class
        .fields
        .iter()
        .find(|field| field.name == field_name)
        .ok_or_else(|| {
            anyhow!("target layout: {module_name} missing field {class_name}.{field_name}")
        })?;

    let offset = u32::try_from(field.offset).map_err(|_| {
        anyhow!("target layout: negative field {module_name}!{class_name}.{field_name}")
    })?;
    Ok((offset, field.type_name.as_str()))
}

fn resolve_spotted_layout(schemas: &SchemaMap) -> Result<SpottedLayout> {
    const WORD_WIDTH: u32 = 4;
    const WORD_COUNT: u32 = 2;

    let (classes, _) = schemas
        .get("client.dll")
        .ok_or_else(|| anyhow!("spotted layout: missing client.dll schema"))?;
    let pawn = classes
        .iter()
        .find(|class| class.name == "C_CSPlayerPawn")
        .ok_or_else(|| anyhow!("spotted layout: missing class C_CSPlayerPawn"))?;
    let spotted_state = classes
        .iter()
        .find(|class| class.name == "EntitySpottedState_t")
        .ok_or_else(|| anyhow!("spotted layout: missing class EntitySpottedState_t"))?;

    ensure!(
        pawn.size > 0 && spotted_state.size > 0,
        "spotted layout: invalid owner-class size"
    );
    let state_field = pawn
        .fields
        .iter()
        .find(|field| field.name == "m_entitySpottedState")
        .ok_or_else(|| anyhow!("spotted layout: missing C_CSPlayerPawn.m_entitySpottedState"))?;
    ensure!(
        state_field.type_name == "EntitySpottedState_t",
        "spotted layout: C_CSPlayerPawn.m_entitySpottedState has unexpected type {}",
        state_field.type_name
    );
    let mask_field = spotted_state
        .fields
        .iter()
        .find(|field| field.name == "m_bSpottedByMask")
        .ok_or_else(|| anyhow!("spotted layout: missing EntitySpottedState_t.m_bSpottedByMask"))?;
    let compact_mask_type: String = mask_field
        .type_name
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    ensure!(
        matches!(compact_mask_type.as_str(), "uint32[2]" | "uint32_t[2]"),
        "spotted layout: EntitySpottedState_t.m_bSpottedByMask has unexpected type {}",
        mask_field.type_name
    );

    let state_offset = u32::try_from(state_field.offset)
        .map_err(|_| anyhow!("spotted layout: negative m_entitySpottedState offset"))?;
    let mask_offset = u32::try_from(mask_field.offset)
        .map_err(|_| anyhow!("spotted layout: negative m_bSpottedByMask offset"))?;
    let pawn_size = u32::try_from(pawn.size)
        .map_err(|_| anyhow!("spotted layout: invalid C_CSPlayerPawn size"))?;
    let state_size = u32::try_from(spotted_state.size)
        .map_err(|_| anyhow!("spotted layout: invalid EntitySpottedState_t size"))?;
    let mask_width = WORD_WIDTH
        .checked_mul(WORD_COUNT)
        .ok_or_else(|| anyhow!("spotted layout: mask width overflow"))?;

    ensure!(
        state_offset
            .checked_add(state_size)
            .is_some_and(|end| end <= pawn_size),
        "spotted layout: m_entitySpottedState exceeds C_CSPlayerPawn"
    );
    ensure!(
        mask_offset.is_multiple_of(WORD_WIDTH)
            && mask_offset
                .checked_add(mask_width)
                .is_some_and(|end| end <= state_size),
        "spotted layout: m_bSpottedByMask span is invalid"
    );
    let pawn_mask_offset = state_offset
        .checked_add(mask_offset)
        .ok_or_else(|| anyhow!("spotted layout: combined mask offset overflow"))?;
    ensure!(
        pawn_mask_offset.is_multiple_of(WORD_WIDTH)
            && pawn_mask_offset
                .checked_add(mask_width)
                .is_some_and(|end| end <= pawn_size),
        "spotted layout: combined mask span exceeds C_CSPlayerPawn"
    );

    Ok(SpottedLayout {
        pawn_mask_offset,
        word_width: WORD_WIDTH,
        word_count: WORD_COUNT,
    })
}

fn capture_layout_evidence<'a, P: Pe<'a>>(view: P) -> Result<LayoutEvidence> {
    let entity = unique_captures(view, "entity resolver", ENTITY_RESOLVER_PATTERN)?;
    let chunk = unique_captures(view, "entity chunk base", ENTITY_CHUNK_BASE_PATTERN)?;
    let skeleton = unique_captures(view, "skeleton bone array", SKELETON_BONE_ARRAY_PATTERN)?;
    let transform_stride =
        unique_captures(view, "bone transform stride", BONE_TRANSFORM_STRIDE_PATTERN)?;
    let transform = unique_captures(view, "CTransform components", TRANSFORM_COMPONENT_PATTERN)?;
    let model_name = unique_captures(view, "model name symbol", MODEL_NAME_PATTERN)?;

    Ok(LayoutEvidence {
        chunk_pointer_base: capture_u8(&chunk, 1, "entity chunk pointer base")?,
        chunk_pointer_sib: capture_u8(&entity, 3, "entity chunk pointer SIB")?,
        entry_stride: capture_u8(&entity, 5, "entity entry stride")?,
        handle_identity_offset: capture_u8(&entity, 6, "entity handle identity offset")?,
        chunk_shift: capture_u8(&entity, 2, "entity chunk shift")?,
        chunk_index_mask: capture_u32(&entity, 4, "entity chunk index mask")?,
        handle_index_mask: capture_u32(&entity, 1, "entity handle index mask")?,
        skeleton_bone_array_offset: capture_u32(&skeleton, 1, "skeleton bone-array offset")?,
        transform_stride_shift: capture_u8(&transform_stride, 1, "bone transform stride shift")?,
        translation_z_offset: capture_u8(&transform, 1, "transform translation z offset")?,
        quaternion_offset: capture_u8(&transform, 2, "transform quaternion offset")?,
        scale_offset: capture_u8(&transform, 3, "transform scale offset")?,
        skeleton_model_name_offset: capture_u32(&model_name, 1, "skeleton model-name offset")?,
        second_skeleton_model_name_offset: capture_u32(
            &model_name,
            2,
            "second skeleton model-name offset",
        )?,
    })
}

fn resolve_layout_evidence(
    evidence: LayoutEvidence,
    model_state_offset: u32,
    model_name_offset: u32,
) -> Result<TargetLayout> {
    ensure!(
        evidence.handle_index_mask != 0
            && evidence
                .handle_index_mask
                .checked_add(1)
                .is_some_and(u32::is_power_of_two),
        "target layout: entity handle index mask is not contiguous"
    );
    let handle_serial_shift = evidence.handle_index_mask.count_ones() as u8;
    ensure!(
        evidence.chunk_shift < handle_serial_shift,
        "target layout: entity chunk shift does not fit the handle index"
    );
    let chunk_size = 1_u32
        .checked_shl(u32::from(evidence.chunk_shift))
        .ok_or_else(|| anyhow!("target layout: entity chunk shift overflow"))?;
    ensure!(
        evidence.chunk_index_mask.checked_add(1) == Some(chunk_size),
        "target layout: entity chunk mask contradicts the chunk shift"
    );
    let handle_capacity = evidence
        .handle_index_mask
        .checked_add(1)
        .ok_or_else(|| anyhow!("target layout: entity handle capacity overflow"))?;
    ensure!(
        handle_capacity % chunk_size == 0,
        "target layout: entity chunks do not cover the handle index range"
    );
    let chunk_count = handle_capacity / chunk_size;

    ensure!(
        evidence.chunk_pointer_sib & 0x3f == 0,
        "target layout: unexpected entity chunk-pointer addressing"
    );
    let chunk_pointer_stride = 1_u32
        .checked_shl(u32::from(evidence.chunk_pointer_sib >> 6))
        .ok_or_else(|| anyhow!("target layout: entity chunk-pointer scale overflow"))?;
    ensure!(
        chunk_pointer_stride == u64::BITS / 8,
        "target layout: entity chunk-pointer stride is not x64 pointer-sized"
    );
    ensure!(
        u32::from(evidence.chunk_pointer_base) % chunk_pointer_stride == 0,
        "target layout: entity chunk-pointer base is misaligned"
    );
    ensure!(
        evidence.entry_stride != 0 && u32::from(evidence.entry_stride) % chunk_pointer_stride == 0,
        "target layout: entity entry stride is invalid"
    );
    let handle_identity_offset = u32::from(evidence.handle_identity_offset);
    ensure!(
        handle_identity_offset % u32::BITS.div_ceil(8) == 0
            && handle_identity_offset >= chunk_pointer_stride
            && handle_identity_offset
                .checked_add(u32::BITS.div_ceil(8))
                .is_some_and(|end| end <= u32::from(evidence.entry_stride)),
        "target layout: entity handle identity does not fit the entry stride"
    );

    let transform_stride = 1_u32
        .checked_shl(u32::from(evidence.transform_stride_shift))
        .ok_or_else(|| anyhow!("target layout: bone transform stride overflow"))?;
    ensure!(
        transform_stride.is_power_of_two() && (16..=128).contains(&transform_stride),
        "target layout: bone transform stride is implausible"
    );
    // The matched instructions read x/y from [transform], z from +8,
    // quaternion from a captured member, and scalar scale from another member.
    let translation_offset = 0;
    ensure!(
        u32::from(evidence.translation_z_offset) == translation_offset + 8,
        "target layout: transform translation components are not contiguous"
    );
    let quaternion_offset = u32::from(evidence.quaternion_offset);
    let scale_offset = u32::from(evidence.scale_offset);
    ensure!(
        quaternion_offset % 4 == 0
            && scale_offset % 4 == 0
            && quaternion_offset
                .checked_add(16)
                .is_some_and(|end| end <= transform_stride)
            && scale_offset
                .checked_add(4)
                .is_some_and(|end| end <= transform_stride)
            && quaternion_offset != scale_offset,
        "target layout: transform members do not fit the stride"
    );

    let bone_array_offset = evidence
        .skeleton_bone_array_offset
        .checked_sub(model_state_offset)
        .ok_or_else(|| anyhow!("target layout: bone array precedes CModelState"))?;
    ensure!(
        bone_array_offset % chunk_pointer_stride == 0,
        "target layout: CModelState bone-array pointer is misaligned"
    );

    ensure!(
        evidence.skeleton_model_name_offset == evidence.second_skeleton_model_name_offset,
        "target layout: model-name captures disagree"
    );
    let expected_model_name_offset = model_state_offset
        .checked_add(model_name_offset)
        .ok_or_else(|| anyhow!("target layout: model-name offset overflow"))?;
    ensure!(
        evidence.skeleton_model_name_offset == expected_model_name_offset,
        "target layout: model-name code access contradicts SchemaSystem"
    );

    Ok(TargetLayout {
        entity: EntityLayout {
            chunk_pointer_base: u32::from(evidence.chunk_pointer_base),
            chunk_pointer_stride,
            entry_stride: u32::from(evidence.entry_stride),
            handle_identity_offset,
            chunk_size,
            chunk_count,
            handle_index_mask: evidence.handle_index_mask,
            handle_serial_shift,
        },
        skeleton: SkeletonLayout {
            bone_array_offset,
            transform_stride,
            translation_offset,
            quaternion_offset,
            scale_offset,
            pose_generation: None,
        },
        // Both matched accesses load the string pointer directly from the
        // CUtlSymbolLarge field, proving a zero data-member displacement.
        model_name: ModelNameLayout {
            symbol_data_offset: 0,
        },
        spotted: None,
    })
}

fn unique_captures<'a, P: Pe<'a>>(view: P, name: &str, pat: &[Atom]) -> Result<Vec<u32>> {
    let mut matches = view.scanner().matches_code(pat);
    require_unique_capture(name, save_len(pat), |save| matches.next(save))
}

fn require_unique_capture(
    name: &str,
    capture_count: usize,
    mut next: impl FnMut(&mut [u32]) -> bool,
) -> Result<Vec<u32>> {
    let mut capture = vec![0; capture_count];
    if !next(&mut capture) {
        bail!("target layout: required pattern `{name}` is absent");
    }
    let mut duplicate = vec![0; capture_count];
    if next(&mut duplicate) {
        bail!("target layout: required pattern `{name}` is ambiguous");
    }
    Ok(capture)
}

fn capture_u32(captures: &[u32], slot: usize, name: &str) -> Result<u32> {
    captures
        .get(slot)
        .copied()
        .ok_or_else(|| anyhow!("target layout: missing capture `{name}`"))
}

fn capture_u8(captures: &[u32], slot: usize, name: &str) -> Result<u8> {
    u8::try_from(capture_u32(captures, slot, name)?)
        .map_err(|_| anyhow!("target layout: capture `{name}` does not fit u8"))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use pelite::pe64::PeFile;

    use super::{
        LayoutEvidence, capture_layout_evidence, capture_u8, require_unique_capture,
        resolve_layout_evidence, resolve_spotted_layout,
    };
    use crate::analysis::{Class, ClassField};

    fn captures<'a>(rows: &'a [&'a [u32]]) -> impl FnMut(&mut [u32]) -> bool + 'a {
        let mut rows = rows.iter();
        move |save| {
            let Some(row) = rows.next() else {
                return false;
            };
            save.copy_from_slice(row);
            true
        }
    }

    fn current_shape_evidence() -> LayoutEvidence {
        LayoutEvidence {
            chunk_pointer_base: 0x10,
            chunk_pointer_sib: 0xc0,
            entry_stride: 0x70,
            handle_identity_offset: 0x10,
            chunk_shift: 9,
            chunk_index_mask: 0x1ff,
            handle_index_mask: 0x7fff,
            skeleton_bone_array_offset: 0x1c0,
            transform_stride_shift: 5,
            translation_z_offset: 8,
            quaternion_offset: 0x10,
            scale_offset: 0x0c,
            skeleton_model_name_offset: 0x1e8,
            second_skeleton_model_name_offset: 0x1e8,
        }
    }

    fn unique_capture_returns_the_typed_immediate_for_one_match() {
        let rows: &[&[u32]] = &[&[0x1000, 0x1234_5678]];
        let capture = require_unique_capture("fixture", 2, captures(rows)).expect("unique capture");

        assert_eq!(capture[1], 0x1234_5678);
    }

    #[test]
    fn unique_capture_errors_when_pattern_is_absent() {
        let error = require_unique_capture("fixture", 2, captures(&[])).unwrap_err();

        assert!(error.to_string().contains("absent"));
    }

    #[test]
    fn unique_capture_errors_when_pattern_is_ambiguous() {
        let rows: &[&[u32]] = &[&[0x1000, 7], &[0x2000, 7]];
        let error = require_unique_capture("fixture", 2, captures(rows)).unwrap_err();

        assert!(error.to_string().contains("ambiguous"));
    }

    #[test]
    fn capture_u8_rejects_out_of_width_values() {
        assert!(capture_u8(&[0, 256], 1, "fixture").is_err());
    }

    #[test]
    fn resolved_layout_preserves_proven_zero_member_locations() {
        let layout =
            resolve_layout_evidence(current_shape_evidence(), 0x140, 0xa8).expect("valid layout");

        assert_eq!(layout.skeleton.bone_array_offset, 0x80);
        assert_eq!(layout.skeleton.translation_offset, 0);
        assert_eq!(layout.model_name.symbol_data_offset, 0);
        assert_eq!(layout.entity.chunk_count, 64);
        assert_eq!(layout.entity.handle_serial_shift, 15);
        assert_eq!(layout.entity.handle_identity_offset, 0x10);
    }

    #[test]
    fn schema_and_code_model_name_offsets_must_agree() {
        let error = resolve_layout_evidence(current_shape_evidence(), 0x140, 0xac).unwrap_err();

        assert!(error.to_string().contains("contradicts SchemaSystem"));
    }

    #[test]
    fn entity_handle_identity_capture_must_fit_outside_the_entry_pointer() {
        for offset in [0, 4, 0x6d, 0x70] {
            let mut evidence = current_shape_evidence();
            evidence.handle_identity_offset = offset;

            let error = resolve_layout_evidence(evidence, 0x140, 0xa8).unwrap_err();

            assert!(error.to_string().contains("handle identity"));
        }
    }

    #[test]
    fn spotted_layout_uses_proven_field_spans_not_class_alignment_metadata() {
        let class = |name: &str, size: i32, fields: Vec<ClassField>| Class {
            name: name.to_owned(),
            module_name: "client.dll".to_owned(),
            parent_name: None,
            size,
            alignment: 1,
            metadata: Vec::new(),
            fields,
        };
        let schemas = BTreeMap::from([(
            "client.dll".to_owned(),
            (
                vec![
                    class(
                        "C_CSPlayerPawn",
                        0x2000,
                        vec![ClassField {
                            name: "m_entitySpottedState".to_owned(),
                            type_name: "EntitySpottedState_t".to_owned(),
                            offset: 0x1cc8,
                        }],
                    ),
                    class(
                        "EntitySpottedState_t",
                        0x18,
                        vec![ClassField {
                            name: "m_bSpottedByMask".to_owned(),
                            type_name: "uint32[2]".to_owned(),
                            offset: 0x0c,
                        }],
                    ),
                ],
                Vec::new(),
            ),
        )]);

        let layout = resolve_spotted_layout(&schemas).expect("valid field evidence");
        assert_eq!(layout.pawn_mask_offset, 0x1cd4);
    }

    #[test]
    #[ignore = "requires CS2_CLIENT_DLL to point at the installed current client.dll"]
    fn installed_client_patterns_resolve_uniquely() {
        let path = std::env::var_os("CS2_CLIENT_DLL").expect("CS2_CLIENT_DLL");
        let bytes = fs::read(path).expect("read current client.dll");
        let view = PeFile::from_bytes(&bytes).expect("parse current client.dll");

        capture_layout_evidence(view).expect("all current target-layout patterns are unique");
    }
}
