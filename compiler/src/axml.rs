//! Binary Android XML manifest writer. No aapt, no resources.arsc.
//!
//! Attribute names occupy the front of the string pool so the resource map
//! lines up with them. Attributes on an element are sorted by (namespace,
//! resource id), with the null namespace last.

use std::collections::HashMap;

use serde_json::{Number, Value};

const ANDROID_NS: &str = "http://schemas.android.com/apk/res/android";
const RES_NULL: u32 = 0xFFFF_FFFF;

/// name -> android.R.attr id. These strings are pool entries 0..n-1.
const ATTRS: &[(&str, u32)] = &[
    ("theme", 0x01010000),
    ("label", 0x01010001),
    ("icon", 0x01010002),
    ("name", 0x01010003),
    ("permission", 0x01010006),
    ("debuggable", 0x0101000F),
    ("exported", 0x01010010),
    ("minSdkVersion", 0x0101020C),
    ("versionCode", 0x0101021B),
    ("versionName", 0x0101021C),
    ("targetSdkVersion", 0x01010270),
    ("hardwareAccelerated", 0x010102D3),
    ("extractNativeLibs", 0x010104EA),
    ("compileSdkVersion", 0x01010572),
    ("compileSdkVersionCodename", 0x01010573),
];

const TYPE_REFERENCE: u8 = 0x01;
const TYPE_STRING: u8 = 0x03;
const TYPE_INT: u8 = 0x10;
const TYPE_INT_BOOLEAN: u8 = 0x12;

/// Theme.DeviceDefault.Light.NoActionBar (API 14+).
const THEME_DEVICE_DEFAULT_LIGHT_NO_ACTION_BAR: u32 = 16974124;

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn utf16_item(text: &str) -> Vec<u8> {
    let count = text.chars().count();
    if count >= 0x8000 {
        panic!("string too long for the manifest pool");
    }
    let mut out = Vec::with_capacity(2 + text.encode_utf16().count() * 2 + 2);
    push_u16(&mut out, count as u16);
    for unit in text.encode_utf16() {
        push_u16(&mut out, unit);
    }
    push_u16(&mut out, 0);
    out
}

#[derive(Default)]
struct Strings {
    items: Vec<String>,
    index: HashMap<String, u32>,
}

impl Strings {
    fn add(&mut self, text: &str) -> u32 {
        if let Some(&idx) = self.index.get(text) {
            return idx;
        }
        let idx = self.items.len() as u32;
        self.items.push(text.to_string());
        self.index.insert(text.to_string(), idx);
        idx
    }

    fn build(&self) -> Vec<u8> {
        let mut body = Vec::new();
        let mut offsets = Vec::with_capacity(self.items.len());
        for text in &self.items {
            offsets.push(body.len() as u32);
            body.extend(utf16_item(text));
        }
        let pad = (4 - body.len() % 4) % 4;
        body.extend(std::iter::repeat(0).take(pad));
        let strings_start = 28 + 4 * self.items.len();
        let size = strings_start + body.len();
        let mut out = Vec::with_capacity(size);
        push_u16(&mut out, 0x0001);
        push_u16(&mut out, 28);
        push_u32(&mut out, size as u32);
        push_u32(&mut out, self.items.len() as u32);
        push_u32(&mut out, 0);
        push_u32(&mut out, 0);
        push_u32(&mut out, strings_start as u32);
        push_u32(&mut out, 0);
        for off in offsets {
            push_u32(&mut out, off);
        }
        out.extend_from_slice(&body);
        out
    }
}

fn node(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8 + body.len());
    push_u32(&mut payload, 1);
    push_u32(&mut payload, RES_NULL);
    payload.extend_from_slice(body);
    let size = 8 + payload.len();
    let mut out = Vec::with_capacity(8 + payload.len());
    push_u16(&mut out, kind);
    push_u16(&mut out, 16);
    push_u32(&mut out, size as u32);
    out.extend_from_slice(&payload);
    out
}

fn ns_node(kind: u16, prefix: u32, uri: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(8);
    push_u32(&mut body, prefix);
    push_u32(&mut body, uri);
    node(kind, &body)
}

fn end_element(ns: u32, name: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(8);
    push_u32(&mut body, ns);
    push_u32(&mut body, name);
    node(0x0103, &body)
}

struct Attr {
    ns: u32,
    name: u32,
    raw: u32,
    kind: u8,
    data: u32,
    resid: u32,
}

fn element(ns: u32, name: u32, attrs: &[Attr]) -> Vec<u8> {
    let mut ext = Vec::with_capacity(20 + attrs.len() * 20);
    push_u32(&mut ext, ns);
    push_u32(&mut ext, name);
    push_u16(&mut ext, 20);
    push_u16(&mut ext, 20);
    push_u16(&mut ext, attrs.len() as u16);
    push_u16(&mut ext, 0);
    push_u16(&mut ext, 0);
    push_u16(&mut ext, 0);
    for attr in attrs {
        push_u32(&mut ext, attr.ns);
        push_u32(&mut ext, attr.name);
        push_u32(&mut ext, attr.raw);
        push_u16(&mut ext, 8);
        ext.push(0);
        ext.push(attr.kind);
        push_u32(&mut ext, attr.data);
    }
    node(0x0102, &ext)
}

fn sort_attrs(mut attrs: Vec<Attr>) -> Vec<Attr> {
    attrs.sort_by(|a, b| {
        let rank = |attr: &Attr| u8::from(attr.ns == RES_NULL);
        rank(a)
            .cmp(&rank(b))
            .then(a.resid.cmp(&b.resid))
            .then(a.name.cmp(&b.name))
    });
    attrs
}

fn android_attr(
    attr_index: &HashMap<&str, (u32, u32)>,
    uri: u32,
    attr: &str,
    kind: u8,
    data: u32,
    raw: Option<u32>,
) -> Attr {
    let (name, resid) = attr_index[attr];
    Attr {
        ns: uri,
        name,
        raw: raw.unwrap_or(RES_NULL),
        kind,
        data,
        resid,
    }
}

fn plain_attr(pool: &mut Strings, attr_name: &str, text: &str) -> Attr {
    let idx = pool.add(text);
    Attr {
        ns: RES_NULL,
        name: pool.add(attr_name),
        raw: idx,
        kind: TYPE_STRING,
        data: idx,
        resid: RES_NULL,
    }
}

fn json_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number_is_zero(number),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
    }
}

fn number_is_zero(number: &Number) -> bool {
    if let Some(integer) = number.as_i64() {
        integer == 0
    } else if let Some(integer) = number.as_u64() {
        integer == 0
    } else {
        number.as_f64().map(|float| float == 0.0).unwrap_or(true)
    }
}

fn py_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                integer.to_string()
            } else if let Some(integer) = number.as_u64() {
                integer.to_string()
            } else {
                number.to_string()
            }
        }
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

fn json_string(project: &Value, key: &str) -> String {
    project.get(key).map(py_string).unwrap_or_default()
}

fn text_or(project: &Value, key: &str, default: &str) -> String {
    match project.get(key) {
        Some(value) if !json_falsy(value) => py_string(value),
        _ => default.to_string(),
    }
}

fn int_or(project: &Value, key: &str, default: i64) -> i64 {
    match project.get(key) {
        Some(value) if !json_falsy(value) => match value {
            Value::Number(number) => {
                if let Some(integer) = number.as_i64() {
                    integer
                } else if let Some(integer) = number.as_u64() {
                    integer as i64
                } else {
                    number.as_f64().unwrap_or(default as f64) as i64
                }
            }
            Value::String(text) => text.parse().unwrap_or(default),
            Value::Bool(true) => 1,
            _ => default,
        },
        _ => default,
    }
}

fn permissions(project: &Value) -> Vec<String> {
    match project.get("permissions") {
        Some(Value::Array(items)) => items.iter().map(py_string).collect(),
        _ => Vec::new(),
    }
}

fn u32_data(value: i64) -> u32 {
    value as u32
}

pub fn build_manifest(project: &Value, forms: &[String]) -> Vec<u8> {
    let mut pool = Strings::default();
    let mut attr_index: HashMap<&str, (u32, u32)> = HashMap::new();
    for (name, resid) in ATTRS {
        attr_index.insert(name, (pool.add(name), *resid));
    }

    let mut resmap = Vec::with_capacity(ATTRS.len() * 4);
    for (_, resid) in ATTRS {
        push_u32(&mut resmap, *resid);
    }
    let mut resmap_chunk = Vec::with_capacity(8 + resmap.len());
    push_u16(&mut resmap_chunk, 0x0180);
    push_u16(&mut resmap_chunk, 8);
    push_u32(&mut resmap_chunk, (8 + resmap.len()) as u32);
    resmap_chunk.extend_from_slice(&resmap);

    let prefix = pool.add("android");
    let uri = pool.add(ANDROID_NS);
    let package = json_string(project, "package");
    let title = match project.get("title") {
        Some(value) if !json_falsy(value) => py_string(value),
        _ => json_string(project, "name"),
    };
    let main = json_string(project, "mainForm");

    let manifest_name = pool.add("manifest");
    let uses_sdk = pool.add("uses-sdk");
    let uses_perm = pool.add("uses-permission");
    let application = pool.add("application");
    let activity = pool.add("activity");
    let intent_filter = pool.add("intent-filter");
    let action = pool.add("action");
    let category = pool.add("category");

    let ver_name_text = text_or(project, "versionName", "1.0");
    let ver_name = pool.add(&ver_name_text);
    let codename = pool.add("15");
    let label = pool.add(&title);

    let manifest_attrs = sort_attrs(vec![
        android_attr(
            &attr_index,
            uri,
            "versionCode",
            TYPE_INT,
            u32_data(int_or(project, "versionCode", 1)),
            None,
        ),
        android_attr(
            &attr_index,
            uri,
            "versionName",
            TYPE_STRING,
            ver_name,
            Some(ver_name),
        ),
        android_attr(&attr_index, uri, "compileSdkVersion", TYPE_INT, 35, None),
        android_attr(
            &attr_index,
            uri,
            "compileSdkVersionCodename",
            TYPE_STRING,
            codename,
            Some(codename),
        ),
        plain_attr(&mut pool, "package", &package),
    ]);
    let sdk_attrs = sort_attrs(vec![
        android_attr(
            &attr_index,
            uri,
            "minSdkVersion",
            TYPE_INT,
            u32_data(int_or(project, "minSdk", 24)),
            None,
        ),
        android_attr(
            &attr_index,
            uri,
            "targetSdkVersion",
            TYPE_INT,
            u32_data(int_or(project, "targetSdk", 28)),
            None,
        ),
    ]);
    let app_attrs = sort_attrs(vec![
        android_attr(
            &attr_index,
            uri,
            "theme",
            TYPE_REFERENCE,
            THEME_DEVICE_DEFAULT_LIGHT_NO_ACTION_BAR,
            None,
        ),
        android_attr(&attr_index, uri, "label", TYPE_STRING, label, Some(label)),
        android_attr(
            &attr_index,
            uri,
            "debuggable",
            TYPE_INT_BOOLEAN,
            0xFFFF_FFFF,
            None,
        ),
        android_attr(
            &attr_index,
            uri,
            "hardwareAccelerated",
            TYPE_INT_BOOLEAN,
            0xFFFF_FFFF,
            None,
        ),
        android_attr(
            &attr_index,
            uri,
            "extractNativeLibs",
            TYPE_INT_BOOLEAN,
            0xFFFF_FFFF,
            None,
        ),
    ]);

    let mut parts = vec![
        ns_node(0x0100, prefix, uri),
        element(RES_NULL, manifest_name, &manifest_attrs),
        element(RES_NULL, uses_sdk, &sdk_attrs),
        end_element(RES_NULL, uses_sdk),
    ];
    for perm in permissions(project) {
        let perm_s = pool.add(&perm);
        parts.push(element(
            RES_NULL,
            uses_perm,
            &sort_attrs(vec![android_attr(
                &attr_index,
                uri,
                "name",
                TYPE_STRING,
                perm_s,
                Some(perm_s),
            )]),
        ));
        parts.push(end_element(RES_NULL, uses_perm));
    }
    parts.push(element(RES_NULL, application, &app_attrs));
    for form in forms {
        let dotted = format!("{package}.{form}");
        let form_s = pool.add(&dotted);
        let label_text = if form == &main {
            title.clone()
        } else {
            form.clone()
        };
        let label_s = pool.add(&label_text);
        let exported = if form == &main { 0xFFFF_FFFF } else { 0 };
        parts.push(element(
            RES_NULL,
            activity,
            &sort_attrs(vec![
                android_attr(
                    &attr_index,
                    uri,
                    "label",
                    TYPE_STRING,
                    label_s,
                    Some(label_s),
                ),
                android_attr(&attr_index, uri, "name", TYPE_STRING, form_s, Some(form_s)),
                android_attr(
                    &attr_index,
                    uri,
                    "exported",
                    TYPE_INT_BOOLEAN,
                    exported,
                    None,
                ),
            ]),
        ));
        if form == &main {
            let main_act = pool.add("android.intent.action.MAIN");
            let launcher = pool.add("android.intent.category.LAUNCHER");
            parts.push(element(RES_NULL, intent_filter, &[]));
            parts.push(element(
                RES_NULL,
                action,
                &sort_attrs(vec![android_attr(
                    &attr_index,
                    uri,
                    "name",
                    TYPE_STRING,
                    main_act,
                    Some(main_act),
                )]),
            ));
            parts.push(end_element(RES_NULL, action));
            parts.push(element(
                RES_NULL,
                category,
                &sort_attrs(vec![android_attr(
                    &attr_index,
                    uri,
                    "name",
                    TYPE_STRING,
                    launcher,
                    Some(launcher),
                )]),
            ));
            parts.push(end_element(RES_NULL, category));
            parts.push(end_element(RES_NULL, intent_filter));
        }
        parts.push(end_element(RES_NULL, activity));
    }
    parts.push(end_element(RES_NULL, application));
    parts.push(end_element(RES_NULL, manifest_name));
    parts.push(ns_node(0x0101, prefix, uri));

    let string_chunk = pool.build();
    let mut body = Vec::new();
    body.extend_from_slice(&string_chunk);
    body.extend_from_slice(&resmap_chunk);
    for part in &parts {
        body.extend_from_slice(part);
    }
    let size = 8 + body.len();
    let mut out = Vec::with_capacity(size);
    push_u16(&mut out, 0x0003);
    push_u16(&mut out, 8);
    push_u32(&mut out, size as u32);
    out.extend_from_slice(&body);
    out
}
