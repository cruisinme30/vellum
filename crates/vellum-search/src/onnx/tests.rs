// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Runs the embedder against a tiny ONNX model built here: an embedding
//! table looked up by token ID, so pooled vectors are predictable.

use prost::Message as _;
use tract_onnx::pb::tensor_shape_proto::{Dimension, dimension};
use tract_onnx::pb::{
    GraphProto, ModelProto, NodeProto, OperatorSetIdProto, TensorProto, TensorShapeProto,
    TypeProto, ValueInfoProto, type_proto,
};

use super::*;

const FLOAT: i32 = 1;
const INT64: i32 = 7;
const VOCAB: &str = "[PAD]\n[UNK]\n[CLS]\n[SEP]\ngateway\ndeploy\nbudget";
const DIMS: usize = 4;

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
/// specials are zero, `gateway` = e0, `deploy` = e1, `budget` = e2.
fn table() -> Vec<f32> {
    let mut t = vec![0.0; 7 * DIMS];
    t[4 * DIMS] = 1.0;
    t[5 * DIMS + 1] = 1.0;
    t[6 * DIMS + 2] = 1.0;
    t
}

fn write_model(dir: &std::path::Path, with_types: bool) -> OnnxConfig {
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
                dims: vec![7, i64::try_from(DIMS).unwrap()],
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

#[tokio::test]
async fn embeds_mean_pooled_unit_vectors_in_batches_of_mixed_length() {
    let dir = tempfile::tempdir().unwrap();
    let embedder = OnnxEmbedder::new(write_model(dir.path(), true));
    assert_eq!(embedder.dimensions(), DIMS);
    let vectors = embedder
        .embed_passages(&[
            "gateway".into(),
            "gateway deploy deploy".into(),
            String::new(),
        ])
        .await
        .unwrap();
    assert_eq!(vectors.len(), 3);
    // `[CLS] gateway [SEP]`: specials are zero, so the unit vector is e0.
    assert!((vectors[0][0] - 1.0).abs() < 1e-5);
    // One gateway, two deploys: direction (1, 2, 0, 0).
    let norm = 5.0_f32.sqrt();
    assert!((vectors[1][0] - 1.0 / norm).abs() < 1e-5);
    assert!((vectors[1][1] - 2.0 / norm).abs() < 1e-5);
    // Only specials pool to zero, which stays zero.
    assert!(vectors[2].iter().all(|x| x.abs() < 1e-6));
    let query = embedder.embed_query("budget").await.unwrap();
    assert!((query[2] - 1.0).abs() < 1e-5);
}

#[tokio::test]
async fn works_without_token_type_ids() {
    let dir = tempfile::tempdir().unwrap();
    let embedder = OnnxEmbedder::new(write_model(dir.path(), false));
    let v = embedder.embed_query("deploy").await.unwrap();
    assert!((v[1] - 1.0).abs() < 1e-5);
}

#[tokio::test]
async fn wrong_dimensions_and_missing_files_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = write_model(dir.path(), true);
    config.dimensions = 8;
    assert!(
        OnnxEmbedder::new(config.clone())
            .embed_query("gateway")
            .await
            .is_err()
    );
    config.model = dir.path().join("missing.onnx");
    assert!(
        OnnxEmbedder::new(config)
            .embed_query("gateway")
            .await
            .is_err()
    );
}
