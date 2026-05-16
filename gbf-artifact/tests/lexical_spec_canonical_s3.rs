use gbf_artifact::{
    BOS_ID, CHARSET_V1, Char, ControlTokenName, EOS_ID, LexicalSpec_v1, NormalizationSpec_v1,
    ReservedIdPolicy, UNK_ID,
};

#[test]
fn lexical_spec_v1_canonical_bytes_are_pinned() {
    let spec = LexicalSpec_v1::pinned();
    let canonical = String::from_utf8(spec.canonical_bytes()).expect("canonical bytes are UTF-8");

    insta::assert_snapshot!(&canonical, @r###"{"charset":[{"codepoint":"A","id":0,"kind":"printable"},{"codepoint":"B","id":1,"kind":"printable"},{"codepoint":"C","id":2,"kind":"printable"},{"codepoint":"D","id":3,"kind":"printable"},{"codepoint":"E","id":4,"kind":"printable"},{"codepoint":"F","id":5,"kind":"printable"},{"codepoint":"G","id":6,"kind":"printable"},{"codepoint":"H","id":7,"kind":"printable"},{"codepoint":"I","id":8,"kind":"printable"},{"codepoint":"J","id":9,"kind":"printable"},{"codepoint":"K","id":10,"kind":"printable"},{"codepoint":"L","id":11,"kind":"printable"},{"codepoint":"M","id":12,"kind":"printable"},{"codepoint":"N","id":13,"kind":"printable"},{"codepoint":"O","id":14,"kind":"printable"},{"codepoint":"P","id":15,"kind":"printable"},{"codepoint":"Q","id":16,"kind":"printable"},{"codepoint":"R","id":17,"kind":"printable"},{"codepoint":"S","id":18,"kind":"printable"},{"codepoint":"T","id":19,"kind":"printable"},{"codepoint":"U","id":20,"kind":"printable"},{"codepoint":"V","id":21,"kind":"printable"},{"codepoint":"W","id":22,"kind":"printable"},{"codepoint":"X","id":23,"kind":"printable"},{"codepoint":"Y","id":24,"kind":"printable"},{"codepoint":"Z","id":25,"kind":"printable"},{"codepoint":"a","id":26,"kind":"printable"},{"codepoint":"b","id":27,"kind":"printable"},{"codepoint":"c","id":28,"kind":"printable"},{"codepoint":"d","id":29,"kind":"printable"},{"codepoint":"e","id":30,"kind":"printable"},{"codepoint":"f","id":31,"kind":"printable"},{"codepoint":"g","id":32,"kind":"printable"},{"codepoint":"h","id":33,"kind":"printable"},{"codepoint":"i","id":34,"kind":"printable"},{"codepoint":"j","id":35,"kind":"printable"},{"codepoint":"k","id":36,"kind":"printable"},{"codepoint":"l","id":37,"kind":"printable"},{"codepoint":"m","id":38,"kind":"printable"},{"codepoint":"n","id":39,"kind":"printable"},{"codepoint":"o","id":40,"kind":"printable"},{"codepoint":"p","id":41,"kind":"printable"},{"codepoint":"q","id":42,"kind":"printable"},{"codepoint":"r","id":43,"kind":"printable"},{"codepoint":"s","id":44,"kind":"printable"},{"codepoint":"t","id":45,"kind":"printable"},{"codepoint":"u","id":46,"kind":"printable"},{"codepoint":"v","id":47,"kind":"printable"},{"codepoint":"w","id":48,"kind":"printable"},{"codepoint":"x","id":49,"kind":"printable"},{"codepoint":"y","id":50,"kind":"printable"},{"codepoint":"z","id":51,"kind":"printable"},{"codepoint":"0","id":52,"kind":"printable"},{"codepoint":"1","id":53,"kind":"printable"},{"codepoint":"2","id":54,"kind":"printable"},{"codepoint":"3","id":55,"kind":"printable"},{"codepoint":"4","id":56,"kind":"printable"},{"codepoint":"5","id":57,"kind":"printable"},{"codepoint":"6","id":58,"kind":"printable"},{"codepoint":"7","id":59,"kind":"printable"},{"codepoint":"8","id":60,"kind":"printable"},{"codepoint":"9","id":61,"kind":"printable"},{"codepoint":" ","id":62,"kind":"printable"},{"codepoint":".","id":63,"kind":"printable"},{"codepoint":",","id":64,"kind":"printable"},{"codepoint":"!","id":65,"kind":"printable"},{"codepoint":"?","id":66,"kind":"printable"},{"codepoint":"-","id":67,"kind":"printable"},{"codepoint":"'","id":68,"kind":"printable"},{"codepoint":":","id":69,"kind":"printable"},{"codepoint":";","id":70,"kind":"printable"},{"codepoint":"(","id":71,"kind":"printable"},{"codepoint":")","id":72,"kind":"printable"},{"codepoint":"\"","id":73,"kind":"printable"},{"codepoint":"/","id":74,"kind":"printable"},{"codepoint":"\n","id":75,"kind":"printable"},{"id":76,"kind":"reserved"},{"id":77,"kind":"control","token":"bos"},{"id":78,"kind":"control","token":"eos"},{"id":79,"kind":"control","token":"unk"}],"control_tokens":{"bos":77,"eos":78,"unk":79},"lexical_self_hash":"sha256:4d9c3b4a648be05b8a60fdd29e416f7c7963b75aa07a1d7ad0ad3fa6d6343984","normalization":{"max_unmappable_pct_per_example":2.0,"normalization_self_hash":"sha256:17de7030da3b68adb7bf6a00aeb060aa569875c7aa1883145b7df599ebf472ca","order":["nfc","strip_combining_accents","preserve_case","fold_quotes_and_dashes","whitespace","unmappable"],"reserved_id_in_input_policy":"reject","schema":"normalization_spec.v1"},"schema":"lexical_spec.v1"}"###);
}

#[test]
fn lexical_and_normalization_specs_round_trip_through_serde() {
    let lexical = LexicalSpec_v1::pinned();
    let encoded = serde_json::to_string(&lexical).expect("lexical spec serializes");
    let decoded: LexicalSpec_v1 = serde_json::from_str(&encoded).expect("lexical spec decodes");
    assert_eq!(decoded, lexical);
    assert_eq!(decoded.lexical_self_hash, decoded.compute_self_hash());

    let normalization = NormalizationSpec_v1::pinned();
    let encoded = serde_json::to_string(&normalization).expect("normalization spec serializes");
    let decoded: NormalizationSpec_v1 =
        serde_json::from_str(&encoded).expect("normalization spec decodes");
    assert_eq!(decoded, normalization);
    assert_eq!(decoded.normalization_self_hash, decoded.compute_self_hash());
}

#[test]
fn charset_v1_table_pins_rfc_boundaries() {
    assert_eq!(CHARSET_V1.len(), 80);
    assert_eq!(
        CHARSET_V1[0],
        Char::Printable {
            id: 0,
            codepoint: 'A'
        }
    );
    assert_eq!(
        CHARSET_V1[25],
        Char::Printable {
            id: 25,
            codepoint: 'Z'
        }
    );
    assert_eq!(
        CHARSET_V1[26],
        Char::Printable {
            id: 26,
            codepoint: 'a'
        }
    );
    assert_eq!(
        CHARSET_V1[51],
        Char::Printable {
            id: 51,
            codepoint: 'z'
        }
    );
    assert_eq!(
        CHARSET_V1[62],
        Char::Printable {
            id: 62,
            codepoint: ' '
        }
    );
    assert_eq!(
        CHARSET_V1[75],
        Char::Printable {
            id: 75,
            codepoint: '\n'
        }
    );
    assert_eq!(CHARSET_V1[76], Char::Reserved { id: 76 });
    assert_eq!(
        CHARSET_V1[77],
        Char::Control {
            id: BOS_ID,
            token: ControlTokenName::Bos
        }
    );
    assert_eq!(
        CHARSET_V1[78],
        Char::Control {
            id: EOS_ID,
            token: ControlTokenName::Eos
        }
    );
    assert_eq!(
        CHARSET_V1[79],
        Char::Control {
            id: UNK_ID,
            token: ControlTokenName::Unk
        }
    );
}

#[test]
fn normalization_spec_rejects_order_drift_on_deserialize() {
    let mut value =
        serde_json::to_value(NormalizationSpec_v1::pinned()).expect("normalization spec value");
    value["order"][0] = serde_json::json!("preserve_case");

    let err = serde_json::from_value::<NormalizationSpec_v1>(value)
        .expect_err("normalization order drift rejects");
    assert!(err.to_string().contains("normalization order"));

    assert_eq!(
        NormalizationSpec_v1::pinned().reserved_id_in_input_policy,
        ReservedIdPolicy::Reject
    );
}

#[test]
fn specs_reject_schema_literal_drift_on_deserialize() {
    let mut lexical = serde_json::to_value(LexicalSpec_v1::pinned()).expect("lexical spec value");
    lexical["schema"] = serde_json::json!("lexical_spec.v2");
    let err = serde_json::from_value::<LexicalSpec_v1>(lexical)
        .expect_err("lexical schema drift rejects");
    assert!(err.to_string().contains("lexical_spec.v1"));

    let mut normalization =
        serde_json::to_value(NormalizationSpec_v1::pinned()).expect("normalization spec value");
    normalization["schema"] = serde_json::json!("normalization_spec.v2");
    let err = serde_json::from_value::<NormalizationSpec_v1>(normalization)
        .expect_err("normalization schema drift rejects");
    assert!(err.to_string().contains("normalization_spec.v1"));
}
