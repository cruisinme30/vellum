// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A tiny ONNX embedding model for tests: an embedding table looked up
//! by token ID, so pooled vectors are predictable.

#![allow(
    clippy::unwrap_used,
    clippy::missing_panics_doc,
    clippy::indexing_slicing
)]

use prost::Message as _;
use tract_onnx::pb::tensor_shape_proto::{Dimension, dimension};
use tract_onnx::pb::{
    GraphProto, ModelProto, NodeProto, OperatorSetIdProto, TensorProto,
    TensorShapeProto, TypeProto, ValueInfoProto, type_proto,
};

use super::OnnxConfig;

const FLOAT: i32 = 1;
const INT64: i32 = 7;
const VOCAB: &str =
    "[PAD]\n[UNK]\n[CLS]\n[SEP]\ngateway\ndeploy\nbudget\nrollout";
const ROWS: usize = 8;
/// The tiny model's vector size.
pub const DIMS: usize = 4;

fn value(name: &str, elem_type: i32, dims: &[Dimension]) -> ValueInfoProto {
    ValueInfoProto {
        name: name.into(),
        r#type: Some(TypeProto {
            value: Some(type_proto::Value::TensorType(type_proto::Tensor {
                elem_type,
                shape: Some(TensorShapeProto { dim: dims.to_vec() }),
            })),
            ..TypeProto::default()
        }),
        ..ValueInfoProto::default()
    }
}

fn param(name: &str) -> Dimension {
    Dimension {
        value: Some(dimension::Value::DimParam(name.into())),
        ..Dimension::default()
    }
}

fn fixed(n: i64) -> Dimension {
    Dimension {
        value: Some(dimension::Value::DimValue(n)),
        ..Dimension::default()
    }
}

/// Token i maps to a one-hot-ish row so pooling is easy to predict:
/// specials are zero, `gateway` = e0, `deploy` = e1, `budget` = e2, and
/// `rollout` = e1 too: a synonym of `deploy` that shares no letters with
/// it, for semantic-only matches.
fn table() -> Vec<f32> {
    let mut t = vec![0.0; ROWS * DIMS];
    t[4 * DIMS] = 1.0;
    t[5 * DIMS + 1] = 1.0;
    t[6 * DIMS + 2] = 1.0;
    t[7 * DIMS + 1] = 1.0;
    t
}

/// Writes the tiny model, its vocabulary and a `config.json` to `dir`.
/// Its words are `gateway`, `deploy` and `budget`, each its own axis, and
/// `rollout`, which means the same as `deploy`.
///
/// # Panics
/// When `dir` cannot be written.
#[must_use]
pub fn write_model(dir: &std::path::Path, with_types: bool) -> OnnxConfig {
    let mut inputs = vec![
        value("input_ids", INT64, &[param("batch"), param("seq")]),
        value("attention_mask", INT64, &[param("batch"), param("seq")]),
    ];
    if with_types {
        inputs.push(value(
            "token_type_ids",
            INT64,
            &[param("batch"), param("seq")],
        ));
    }
    let model = ModelProto {
        ir_version: 8,
        opset_import: vec![OperatorSetIdProto {
            domain: String::new(),
            version: 13,
        }],
        graph: Some(GraphProto {
            name: "tiny".into(),
            node: vec![NodeProto {
                input: vec!["table".into(), "input_ids".into()],
                output: vec!["last_hidden_state".into()],
                name: "lookup".into(),
                op_type: "Gather".into(),
                ..NodeProto::default()
            }],
            initializer: vec![TensorProto {
                name: "table".into(),
                dims: vec![
                    i64::try_from(ROWS).unwrap(),
                    i64::try_from(DIMS).unwrap(),
                ],
                data_type: FLOAT,
                float_data: table(),
                ..TensorProto::default()
            }],
            input: inputs,
            output: vec![value(
                "last_hidden_state",
                FLOAT,
                &[
                    param("batch"),
                    param("seq"),
                    fixed(i64::try_from(DIMS).unwrap()),
                ],
            )],
            ..GraphProto::default()
        }),
        ..ModelProto::default()
    };
    let model_path = dir.join("model.onnx");
    let vocab_path = dir.join("vocab.txt");
    std::fs::write(&model_path, model.encode_to_vec()).unwrap();
    std::fs::write(&vocab_path, VOCAB).unwrap();
    std::fs::write(
        dir.join("config.json"),
        format!(concat!(
            r#"{{"_name_or_path": "tiny", "hidden_size": {DIMS}, "#,
            r#""max_position_embeddings": 16}}"#,
        )),
    )
    .unwrap();
    OnnxConfig {
        model: model_path,
        vocab: vocab_path,
        model_id: "tiny".into(),
        dimensions: DIMS,
        max_tokens: 16,
        query_prefix: String::new(),
        passage_prefix: String::new(),
    }
}
