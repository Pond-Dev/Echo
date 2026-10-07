#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::{Context, Result, bail};
use memflow::prelude::v1::*;

mod analysis;
mod memory;
mod output;
mod source2;

pub struct DumpDocuments {
    pub offsets_json: Vec<u8>,
    pub schemas_json: Vec<u8>,
    pub target_layout_json: Vec<u8>,
}

pub fn dump(process_name: &str) -> Result<DumpDocuments> {
    #[cfg(windows)]
    {
        let mut os = memflow_native::create_os(&OsArgs::default(), LibArc::default())
            .context("creating native memflow OS")?;
        let mut process = os
            .process_by_name(process_name)
            .with_context(|| format!("opening {process_name}"))?;
        let result = analysis::analyze_all(&mut process)?;
        documents_from_analysis(&result)
    }

    #[cfg(not(windows))]
    {
        let _ = process_name;
        bail!("cs2-dumper is only supported on Windows")
    }
}

fn documents_from_analysis(result: &analysis::AnalysisResult) -> Result<DumpDocuments> {
    output::documents_from_analysis(result)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;

    use crate::analysis::{
        AnalysisResult, Class, EntityLayout, Enum, EnumMember, ModelNameLayout, OffsetMap,
        SchemaMap, SkeletonLayout, TargetLayout,
    };
    use crate::documents_from_analysis;

    fn complete_target_layout() -> TargetLayout {
        TargetLayout {
            entity: EntityLayout {
                chunk_pointer_base: 0x10,
                chunk_pointer_stride: 8,
                entry_stride: 0x70,
                handle_identity_offset: 0x10,
                chunk_size: 512,
                chunk_count: 64,
                handle_index_mask: 0x7fff,
                handle_serial_shift: 15,
            },
            skeleton: SkeletonLayout {
                bone_array_offset: 0x80,
                transform_stride: 0x20,
                translation_offset: 0,
                quaternion_offset: 0x10,
                scale_offset: 0x0c,
                pose_generation: None,
            },
            model_name: ModelNameLayout {
                symbol_data_offset: 0,
            },
            spotted: None,
        }
    }

    #[test]
    fn documents_from_analysis_returns_json() {
        let mut client_offsets = BTreeMap::new();
        client_offsets.insert("dwEntityList".to_string(), 0x1234);
        let result = AnalysisResult {
            buttons: Default::default(),
            interfaces: Default::default(),
            offsets: OffsetMap::from([("client.dll".to_string(), client_offsets)]),
            schemas: Default::default(),
            target_layout: complete_target_layout(),
        };

        let documents = documents_from_analysis(&result).unwrap();
        let offsets: Value = serde_json::from_slice(&documents.offsets_json).unwrap();
        let schemas: Value = serde_json::from_slice(&documents.schemas_json).unwrap();
        let target_layout: TargetLayout =
            serde_json::from_slice(&documents.target_layout_json).unwrap();

        assert_eq!(offsets["client.dll"]["dwEntityList"], 0x1234);
        assert!(schemas.is_object());
        assert_eq!(target_layout, complete_target_layout());
        assert_eq!(target_layout.model_name.symbol_data_offset, 0);
        assert_eq!(target_layout.entity.handle_identity_offset, 0x10);
    }

    #[test]
    fn schema_document_preserves_class_size_and_alignment() {
        let class = Class {
            name: "CModelState".to_string(),
            module_name: "client.dll".to_string(),
            parent_name: None,
            size: 0x100,
            alignment: 8,
            metadata: Vec::new(),
            fields: Vec::new(),
        };
        let result = AnalysisResult {
            buttons: Default::default(),
            interfaces: Default::default(),
            offsets: Default::default(),
            schemas: SchemaMap::from([("client.dll".to_string(), (vec![class], Vec::new()))]),
            target_layout: complete_target_layout(),
        };

        let documents = documents_from_analysis(&result).unwrap();
        let schemas: Value = serde_json::from_slice(&documents.schemas_json).unwrap();

        assert_eq!(
            schemas["client.dll"]["classes"]["CModelState"]["size"],
            0x100
        );
        assert_eq!(
            schemas["client.dll"]["classes"]["CModelState"]["alignment"],
            8
        );
    }

    #[test]
    fn schema_document_preserves_every_live_module() {
        let server_enum = Enum {
            name: "SolidType_t".to_string(),
            alignment: 1,
            size: 1,
            members: vec![EnumMember {
                name: "SOLID_NONE".to_string(),
                value: 0,
            }],
        };
        let result = AnalysisResult {
            buttons: Default::default(),
            interfaces: Default::default(),
            offsets: Default::default(),
            schemas: SchemaMap::from([
                ("client.dll".to_string(), (Vec::new(), Vec::new())),
                ("server.dll".to_string(), (Vec::new(), vec![server_enum])),
            ]),
            target_layout: complete_target_layout(),
        };

        let documents = documents_from_analysis(&result).unwrap();
        let schemas: Value = serde_json::from_slice(&documents.schemas_json).unwrap();

        assert_eq!(
            schemas["server.dll"]["enums"]["SolidType_t"]["members"]["SOLID_NONE"],
            0
        );
    }
}
