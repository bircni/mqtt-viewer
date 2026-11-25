use std::path::PathBuf;

use prost_reflect::prost::Message as _;
use prost_reflect::{DescriptorPool, DynamicMessage, Value as ProtoValue};
use serde_json::json;

use crate::config::ProtoBinding;
use crate::decode::{Body, Decoder, Format, ProtoRegistry, hex_dump, parse_hex, preview};

const MSGPACK: &str = "83a474656d70cb4035800000000000a26f6bc3a47461677392a16101";
const CBOR: &str = "a26474656d70fb4035800000000000626964182a";
const SPARKPLUG: &str = "08e80712110a0474656d70200a6900000000008035401805";

fn bytes(hex: &str) -> Vec<u8> {
    parse_hex(hex).unwrap()
}

#[test]
fn formats_have_labels() {
    let labels: Vec<&str> = Format::ALL.iter().map(|format| format.label()).collect();
    assert_eq!(labels, ["Auto", "Text", "JSON", "Hex", "Base64", "MessagePack", "CBOR", "Protobuf", "Sparkplug B"]);
    assert_eq!(serde_json::to_string(&Format::MsgPack).unwrap(), "\"msgpack\"");
}

#[test]
fn auto_detection_picks_the_obvious_format() {
    let decoder = Decoder::new();
    assert_eq!(decoder.detect("t", br#"{"a":1}"#), Format::Json);
    assert_eq!(decoder.detect("t", b" [1, 2]"), Format::Json);
    assert_eq!(decoder.detect("t", b"{not json"), Format::Text);
    assert_eq!(decoder.detect("t", b"21.5"), Format::Text, "bare scalars read better as text");
    assert_eq!(decoder.detect("t", "héllo\n".as_bytes()), Format::Text);
    assert_eq!(decoder.detect("t", b""), Format::Text);
    assert_eq!(decoder.detect("t", b"\x00\x01\x02"), Format::Hex);
    assert_eq!(decoder.detect("t", b"\xff\xfe"), Format::Hex);
    assert_eq!(decoder.detect("spBv1.0/group/NDATA/node", &bytes(SPARKPLUG)), Format::Sparkplug);
    assert_eq!(decoder.detect("spBv1.0/group/STATE", b"ONLINE"), Format::Text);
}

#[test]
fn text_and_json_decode() {
    let decoder = Decoder::new();
    let text = decoder.decode("t", b"hello", Format::Auto);
    assert_eq!((text.format, &text.body), (Format::Text, &Body::Text("hello".into())));
    assert_eq!(text.display(), "hello");
    assert!(text.json().is_none());

    let json = decoder.decode("t", br#"{"b":1,"a":[true,null]}"#, Format::Auto);
    assert_eq!(json.format, Format::Json);
    assert_eq!(json.json(), Some(&json!({"b": 1, "a": [true, null]})));
    insta::assert_snapshot!(json.display(), @r#"
    {
      "a": [
        true,
        null
      ],
      "b": 1
    }
    "#);

    // A scalar is valid JSON when asked for explicitly.
    assert_eq!(decoder.decode("t", b"21.5", Format::Json).json(), Some(&json!(21.5)));
}

#[test]
fn malformed_input_is_an_error_not_a_panic() {
    let decoder = Decoder::new();
    for format in Format::ALL {
        for payload in [&b""[..], b"\xff\xfe\x00", b"{", b"\xc1", b"\x9f", b"!!!"] {
            let decoded = decoder.decode("some/topic", payload, format);
            assert_ne!(decoded.format, Format::Auto);
            let _ = decoded.display();
        }
    }
    let bad_text = decoder.decode("t", b"\xff", Format::Text);
    assert!(bad_text.is_error());
    assert!(bad_text.display().starts_with("Cannot decode as Text"), "{}", bad_text.display());
    assert!(decoder.decode("t", b"{", Format::Json).is_error());
    assert!(decoder.decode("t", b"\x92\x01", Format::MsgPack).is_error(), "truncated array");
    assert!(decoder.decode("t", b"\x01\x02", Format::MsgPack).is_error(), "trailing bytes");
    assert!(decoder.decode("t", b"\x9f", Format::Cbor).is_error());
    assert!(decoder.decode("t", b"\x01\x02", Format::Cbor).is_error(), "trailing bytes");
    assert!(decoder.decode("t", b"!!!", Format::Base64).is_error());
    assert!(decoder.decode("t", b"\x01", Format::Protobuf).is_error(), "no binding");
    assert!(decoder.decode("t", b"\xff\xff\xff", Format::Sparkplug).is_error());
}

#[test]
fn hex_dump_shows_offsets_and_ascii() {
    insta::assert_snapshot!(hex_dump(b"Hello, MQTT viewer!\x00\x01\xff"), @r"
    00000000  48 65 6c 6c 6f 2c 20 4d  51 54 54 20 76 69 65 77  |Hello, MQTT view|
    00000010  65 72 21 00 01 ff                                 |er!...|
    ");
    assert_eq!(hex_dump(b""), "");
    let decoder = Decoder::new();
    let decoded = decoder.decode("t", b"\x00\x01", Format::Auto);
    assert_eq!(decoded.format, Format::Hex);
    assert!(matches!(decoded.body, Body::Hex(_)));
}

#[test]
fn hex_text_parses_back() {
    assert_eq!(parse_hex("48 65 6c6c 0x6F"), Ok(b"Hello".to_vec()));
    assert_eq!(parse_hex(""), Ok(Vec::new()));
    assert!(parse_hex("abc").unwrap_err().contains("even"));
    assert!(parse_hex("zz").unwrap_err().contains("zz"));
    parse_hex("é0").unwrap_err();
}

#[test]
fn base64_unwraps_and_sniffs_the_inner_payload() {
    let decoder = Decoder::new();
    let json = decoder.decode("t", b"eyJhIjoxfQ==\n", Format::Base64);
    assert_eq!((json.format, json.json()), (Format::Base64, Some(&json!({"a": 1}))));
    assert_eq!(decoder.decode("t", b"aGVsbG8=", Format::Base64).body, Body::Text("hello".into()));
    assert!(matches!(decoder.decode("t", b"AAEC", Format::Base64).body, Body::Hex(_)));
}

#[test]
fn msgpack_decodes_to_json() {
    let decoder = Decoder::new();
    let decoded = decoder.decode("t", &bytes(MSGPACK), Format::MsgPack);
    assert_eq!(decoded.json(), Some(&json!({"temp": 21.5, "ok": true, "tags": ["a", 1]})));

    // nil, negative int, f32, bin, non-string key, ext, NaN
    let exotic = bytes(
        "88 a36e696c c0 a16e ff a166 ca3fc00000 a162 c4020001 2a a16b a165 d4050a a17a cb7ff8000000000000 a175 cfffffffffffffffff",
    );
    let decoded = decoder.decode("t", &exotic, Format::MsgPack);
    assert_eq!(
        decoded.json(),
        Some(&json!({
            "nil": null, "n": -1, "f": 1.5, "b": "AAE=", "42": "k",
            "e": {"ext_type": 5, "data": "Cg=="}, "z": "NaN", "u": u64::MAX
        }))
    );
}

#[test]
fn cbor_decodes_to_json() {
    let decoder = Decoder::new();
    let decoded = decoder.decode("t", &bytes(CBOR), Format::Cbor);
    assert_eq!(decoded.json(), Some(&json!({"temp": 21.5, "id": 42})));

    // bytes, negative, bool, null, tagged value, integer key, array, big integer beyond u64
    let exotic = bytes(
        "a8 6162 420001 616e 20 6174 f5 617a f6 6164 c1 1a514b67b0 01 6131 6161 8102 6168 c249010000000000000000",
    );
    let decoded = decoder.decode("t", &exotic, Format::Cbor);
    assert_eq!(
        decoded.json(),
        Some(&json!({
            "b": "AAE=", "n": -1, "t": true, "z": null, "d": 1_363_896_240, "1": "1", "a": [2], "h": "AQAAAAAAAAAA"
        }))
    );
}

#[test]
fn sparkplug_payloads_decode_with_the_embedded_schema() {
    let decoder = Decoder::new();
    let decoded = decoder.decode("spBv1.0/plant/NDATA/node1", &bytes(SPARKPLUG), Format::Auto);
    assert_eq!(decoded.format, Format::Sparkplug);
    insta::assert_snapshot!(decoded.display(), @r#"
    {
      "metrics": [
        {
          "datatype": 10,
          "doubleValue": 21.5,
          "name": "temp"
        }
      ],
      "seq": 5,
      "timestamp": 1000
    }
    "#);
}

fn write_proto(dir: &std::path::Path) -> PathBuf {
    std::fs::write(
        dir.join("common.proto"),
        "syntax = \"proto3\";\npackage acme;\nmessage Location { double lat = 1; double lon = 2; }\n",
    )
    .unwrap();
    let path = dir.join("telemetry.proto");
    std::fs::write(
        &path,
        "syntax = \"proto3\";\npackage acme;\nimport \"common.proto\";\n\
         message Telemetry { string device = 1; uint64 uptime = 2; repeated float samples = 3; Location where = 4; }\n",
    )
    .unwrap();
    path
}

fn encode_telemetry(pool: &DescriptorPool) -> Vec<u8> {
    let mut message = DynamicMessage::new(pool.get_message_by_name("acme.Telemetry").unwrap());
    message.set_field_by_name("device", ProtoValue::String("pump-1".into()));
    message.set_field_by_name("uptime", ProtoValue::U64(86_400));
    message.set_field_by_name("samples", ProtoValue::List(vec![ProtoValue::F32(1.5), ProtoValue::F32(2.5)]));
    let mut location = DynamicMessage::new(pool.get_message_by_name("acme.Location").unwrap());
    location.set_field_by_name("lat", ProtoValue::F64(48.4));
    message.set_field_by_name("where", ProtoValue::Message(location));
    message.encode_to_vec()
}

#[test]
fn protobuf_decodes_with_user_schemas_and_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let proto = write_proto(dir.path());
    let bindings = [ProtoBinding { pattern: "devices/+/telemetry".into(), message_type: "acme.Telemetry".into() }];
    let (registry, errors) = ProtoRegistry::load(&[proto.clone(), proto.clone()], &bindings);
    assert_eq!(errors, Vec::<String>::new(), "loading the same file twice is fine");
    assert_eq!(registry.message_types(), ["acme.Location", "acme.Telemetry"]);
    assert_eq!(registry.binding_for("devices/pump-1/telemetry"), Some("acme.Telemetry"));
    assert_eq!(registry.binding_for("devices/pump-1/state"), None);

    let mut compiler = protox::Compiler::new([dir.path()]).unwrap();
    compiler.open_file(&proto).unwrap();
    let payload = encode_telemetry(&compiler.descriptor_pool());

    let decoder = Decoder::with_registry(registry);
    assert_eq!(decoder.registry().message_types().len(), 2);
    let decoded = decoder.decode("devices/pump-1/telemetry", &payload, Format::Auto);
    assert_eq!(decoded.format, Format::Protobuf);
    assert_eq!(
        decoded.json(),
        Some(&json!({"device": "pump-1", "uptime": 86_400, "samples": [1.5, 2.5], "where": {"lat": 48.4}}))
    );
    assert!(decoder.decode("devices/pump-1/telemetry", b"\xff\xff", Format::Protobuf).is_error());
    assert!(decoder.decode("elsewhere", &payload, Format::Protobuf).display().contains("no Protobuf message type"));
}

#[test]
fn broken_schemas_are_reported_and_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let good = write_proto(dir.path());
    let broken = dir.path().join("broken.proto");
    std::fs::write(&broken, "syntax = \"proto3\";\nmessage {").unwrap();
    let missing = dir.path().join("missing.proto");
    let bindings = [ProtoBinding { pattern: "#".into(), message_type: "acme.Nope".into() }];
    let (registry, errors) = ProtoRegistry::load(&[broken, good, missing, PathBuf::from("bare.proto")], &bindings);
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors[0].contains("broken.proto"), "{errors:?}");
    assert_eq!(registry.message_types().len(), 2);
    let error = registry.decode("any", b"").unwrap_err();
    assert!(error.contains("acme.Nope"), "{error}");
}

#[test]
fn previews_are_short_single_lines() {
    assert_eq!(preview(b"", 10), "");
    assert_eq!(preview(b"hello", 10), "hello");
    assert_eq!(preview(b"{\n  \"a\":   1\n}", 20), "{ \"a\": 1 }");
    assert_eq!(preview(b"0123456789abcdef", 10), "0123456789…");
    assert_eq!(preview("ääääääääääää".as_bytes(), 3), "äää…");
    assert_eq!(preview(b"\x00\x01\x02\x03", 9), "00 01 02 (4 bytes)");
    assert_eq!(preview(b"\xff\xfe", 30), "ff fe (2 bytes)");
    // A window that cuts a multi-byte character in half is still text.
    assert_eq!(preview("aé".repeat(50).as_bytes(), 1), "a…");
    let long = "x".repeat(500);
    assert_eq!(preview(long.as_bytes(), 100).chars().count(), 101);
}
