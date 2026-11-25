use bytes::Bytes;
use mqtt_viewer_core::message::{Properties, PublishRequest, QoS};

use crate::forms::{PayloadMode, PublishForm, check_xml};

fn form(topic: &str, payload: &str, mode: PayloadMode) -> PublishForm {
    PublishForm { topic: topic.into(), payload: payload.into(), mode, ..PublishForm::default() }
}

#[test]
fn modes_have_labels() {
    assert_eq!(PayloadMode::ALL.map(PayloadMode::label), ["raw", "json", "xml", "hex"]);
}

#[test]
fn raw_payloads_are_sent_as_typed() {
    let request = form(" a/b ", "hello", PayloadMode::Raw).to_request().unwrap();
    assert_eq!(request.topic, "a/b");
    assert_eq!(&request.payload[..], b"hello");
    assert!(request.properties.is_empty());
}

#[test]
fn topics_and_payloads_are_validated() {
    assert!(form("", "x", PayloadMode::Raw).to_request().unwrap_err().contains("empty"));
    assert!(form("a/#", "x", PayloadMode::Raw).to_request().unwrap_err().contains("wildcard"));
    assert!(form("a", "{", PayloadMode::Json).to_request().unwrap_err().starts_with("invalid JSON"));
    assert!(form("a", "<a>", PayloadMode::Xml).to_request().unwrap_err().starts_with("invalid XML"));
    assert!(form("a", "zz", PayloadMode::Hex).to_request().unwrap_err().contains("hex"));
    form("a", r#"{"ok":true}"#, PayloadMode::Json).to_request().unwrap();
    form("a", "<a><b/></a>", PayloadMode::Xml).to_request().unwrap();
    assert_eq!(&form("a", "00 ff 10", PayloadMode::Hex).to_request().unwrap().payload[..], [0x00, 0xff, 0x10]);
}

#[test]
fn properties_are_built_from_the_filled_fields() {
    let mut filled = form("a", "x", PayloadMode::Raw);
    filled.qos = QoS::ExactlyOnce;
    filled.retain = true;
    filled.user_properties = vec![("k".into(), "v".into()), (String::new(), "ignored".into())];
    filled.content_type = "text/plain".into();
    filled.response_topic = "reply/to".into();
    filled.correlation_data = "id-1".into();
    filled.expiry_secs = " 60 ".into();
    filled.mark_utf8 = true;
    let request = filled.to_request().unwrap();
    assert_eq!(request.qos, QoS::ExactlyOnce);
    assert!(request.retain);
    assert_eq!(
        request.properties,
        Properties {
            user_properties: vec![("k".into(), "v".into())],
            content_type: Some("text/plain".into()),
            response_topic: Some("reply/to".into()),
            correlation_data: Some(Bytes::from_static(b"id-1")),
            message_expiry_secs: Some(60),
            payload_is_utf8: Some(true),
        }
    );

    filled.expiry_secs = "soon".into();
    assert!(filled.to_request().unwrap_err().contains("expiry"));
    filled.expiry_secs.clear();
    filled.response_topic = "a/+".into();
    assert!(filled.to_request().unwrap_err().starts_with("response topic"));
}

#[test]
fn json_payloads_can_be_pretty_printed() {
    let mut json = form("a", r#"{"b":[1,2]}"#, PayloadMode::Json);
    json.format_json().unwrap();
    assert_eq!(json.payload, "{\n  \"b\": [\n    1,\n    2\n  ]\n}");
    let mut broken = form("a", "{", PayloadMode::Json);
    assert!(broken.format_json().unwrap_err().starts_with("invalid JSON"));
    assert_eq!(broken.payload, "{");
}

#[test]
fn requests_load_back_into_the_form() {
    let mut filled = form("a/b", "payload", PayloadMode::Json);
    filled.user_properties = vec![("k".into(), "v".into())];
    filled.content_type = "c".into();
    filled.response_topic = "r".into();
    filled.correlation_data = "d".into();
    filled.expiry_secs = "5".into();
    filled.mark_utf8 = true;
    filled.payload = "{}".into();
    let request = filled.to_request().unwrap();
    let mut loaded = PublishForm { mode: PayloadMode::Json, ..PublishForm::default() };
    loaded.load(&request);
    assert_eq!(loaded, filled);

    // Binary payloads switch the form to hex; text payloads switch it back.
    let binary = PublishRequest { topic: "bin".into(), payload: Bytes::from_static(b"\x00\xff"), ..Default::default() };
    loaded.load(&binary);
    assert_eq!((loaded.mode, loaded.payload.as_str()), (PayloadMode::Hex, "00 ff"));
    assert_eq!(loaded.to_request().unwrap(), binary);
    loaded.load(&PublishRequest { topic: "t".into(), payload: "text".into(), ..Default::default() });
    assert_eq!((loaded.mode, loaded.payload.as_str()), (PayloadMode::Raw, "text"));
}

#[test]
fn xml_well_formedness_is_checked() {
    for good in [
        "<a/>",
        "<a></a>",
        "<?xml version=\"1.0\"?>\n<root attr=\"1\"><child>text</child><empty /></root>\n",
        "<!-- note --><a><![CDATA[ <not a tag> ]]></a>",
        "<!DOCTYPE a><a>1 &lt; 2</a>",
    ] {
        assert_eq!(check_xml(good), Ok(()), "{good}");
    }
    let bad = [
        ("", "no element"),
        ("just text", "no element"),
        ("<a>", "never closed"),
        ("<a></b>", "expected </a>"),
        ("</a>", "unexpected </a>"),
        ("<a", "unterminated tag"),
        ("<a/><b/>", "more than one root"),
        ("<a/>trailing", "text after the root"),
        ("<a/> x <b/>", "text after the root"),
        ("<>", "without a name"),
        ("<a><!-- open</a>", "unterminated comment"),
        ("<a><![CDATA[ open</a>", "unterminated CDATA"),
    ];
    for (text, reason) in bad {
        let error = check_xml(text).unwrap_err();
        assert!(error.contains(reason), "{text:?}: {error}");
    }
}
