//! Widget catalog. Generated from the same tables the designer uses.
use serde_json::{json, Value};

#[derive(Clone, Copy)]
pub struct WidgetSpec {
    pub name: &'static str,
    pub android: &'static str,
    pub desc: &'static str,
    pub group: &'static str,
    pub width: i32,
    pub height: i32,
    pub props: &'static [&'static str],
    pub events: &'static [&'static str],
}

pub static WIDGETS: &[WidgetSpec] = &[
    WidgetSpec {
        name: "Label",
        android: "android.widget.TextView",
        desc: "Landroid/widget/TextView;",
        group: "Text",
        width: 200,
        height: 36,
        props: &["Text", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Clock",
        android: "android.widget.TextClock",
        desc: "Landroid/widget/TextClock;",
        group: "Text",
        width: 200,
        height: 36,
        props: &["Format", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Button",
        android: "android.widget.Button",
        desc: "Landroid/widget/Button;",
        group: "Buttons",
        width: 160,
        height: 44,
        props: &["Text", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Toggle",
        android: "android.widget.ToggleButton",
        desc: "Landroid/widget/ToggleButton;",
        group: "Buttons",
        width: 120,
        height: 44,
        props: &["TextOn", "TextOff", "Checked", "TextSize", "TextColor", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Edit",
        android: "android.widget.EditText",
        desc: "Landroid/widget/EditText;",
        group: "Input",
        width: 240,
        height: 44,
        props: &["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Memo",
        android: "android.widget.EditText",
        desc: "Landroid/widget/EditText;",
        group: "Input",
        width: 240,
        height: 96,
        props: &["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Number",
        android: "android.widget.EditText",
        desc: "Landroid/widget/EditText;",
        group: "Input",
        width: 160,
        height: 44,
        props: &["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Password",
        android: "android.widget.EditText",
        desc: "Landroid/widget/EditText;",
        group: "Input",
        width: 240,
        height: 44,
        props: &["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Email",
        android: "android.widget.EditText",
        desc: "Landroid/widget/EditText;",
        group: "Input",
        width: 240,
        height: 44,
        props: &["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Phone",
        android: "android.widget.EditText",
        desc: "Landroid/widget/EditText;",
        group: "Input",
        width: 200,
        height: 44,
        props: &["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Check",
        android: "android.widget.CheckBox",
        desc: "Landroid/widget/CheckBox;",
        group: "Input",
        width: 180,
        height: 36,
        props: &["Text", "Checked", "TextSize", "TextColor", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Radio",
        android: "android.widget.RadioButton",
        desc: "Landroid/widget/RadioButton;",
        group: "Input",
        width: 180,
        height: 36,
        props: &["Text", "Checked", "TextSize", "TextColor", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Switch",
        android: "android.widget.Switch",
        desc: "Landroid/widget/Switch;",
        group: "Input",
        width: 180,
        height: 36,
        props: &["Text", "Checked", "TextSize", "TextColor", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Seek",
        android: "android.widget.SeekBar",
        desc: "Landroid/widget/SeekBar;",
        group: "Range",
        width: 240,
        height: 36,
        props: &["Max", "Progress", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Progress",
        android: "android.widget.ProgressBar",
        desc: "Landroid/widget/ProgressBar;",
        group: "Range",
        width: 240,
        height: 24,
        props: &["Max", "Progress", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Busy",
        android: "android.widget.ProgressBar",
        desc: "Landroid/widget/ProgressBar;",
        group: "Range",
        width: 48,
        height: 48,
        props: &["Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Rating",
        android: "android.widget.RatingBar",
        desc: "Landroid/widget/RatingBar;",
        group: "Range",
        width: 200,
        height: 40,
        props: &["Stars", "Rating", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Picker",
        android: "android.widget.NumberPicker",
        desc: "Landroid/widget/NumberPicker;",
        group: "Pickers",
        width: 120,
        height: 160,
        props: &["MinValue", "MaxValue", "Value", "Wrap", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Time",
        android: "android.widget.TimePicker",
        desc: "Landroid/widget/TimePicker;",
        group: "Pickers",
        width: 240,
        height: 180,
        props: &["Is24Hour", "Hour", "Minute", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Date",
        android: "android.widget.DatePicker",
        desc: "Landroid/widget/DatePicker;",
        group: "Pickers",
        width: 320,
        height: 280,
        props: &["Year", "Month", "Day", "Enabled", "Visible"],
        events: &["OnClick"],
    },
    WidgetSpec {
        name: "Panel",
        android: "android.view.View",
        desc: "Landroid/view/View;",
        group: "Layout",
        width: 200,
        height: 80,
        props: &["Color", "Enabled", "Visible"],
        events: &["OnClick"],
    },
];

pub fn widget(name: &str) -> Option<&'static WidgetSpec> {
    WIDGETS.iter().find(|w| w.name.eq_ignore_ascii_case(name))
}

pub fn default_props(kind: &str) -> Value {
    match kind {
        "Label" => json!({"Text": "Label", "TextSize": 18, "TextColor": "#1A1A1A"}),
        "Clock" => json!({"Format": "HH:mm", "TextSize": 18, "TextColor": "#1A1A1A"}),
        "Button" => json!({"Text": "Button", "TextSize": 15}),
        "Toggle" => json!({"TextOn": "On", "TextOff": "Off", "Checked": false, "TextSize": 15}),
        "Edit" => json!({"Hint": "Type here", "Text": "", "TextSize": 16}),
        "Memo" => json!({"Hint": "Notes", "Text": "", "TextSize": 16}),
        "Number" => json!({"Hint": "0", "Text": "", "TextSize": 16}),
        "Password" => json!({"Hint": "Password", "Text": "", "TextSize": 16}),
        "Email" => json!({"Hint": "name@example.com", "Text": "", "TextSize": 16}),
        "Phone" => json!({"Hint": "Phone", "Text": "", "TextSize": 16}),
        "Check" => json!({"Text": "Check", "Checked": false, "TextSize": 15}),
        "Radio" => json!({"Text": "Radio", "Checked": false, "TextSize": 15}),
        "Switch" => json!({"Text": "Switch", "Checked": false, "TextSize": 15}),
        "Seek" => json!({"Max": 100, "Progress": 40}),
        "Progress" => json!({"Max": 100, "Progress": 40}),
        "Busy" => json!({}),
        "Rating" => json!({"Stars": 5, "Rating": 3}),
        "Picker" => json!({"MinValue": 0, "MaxValue": 10, "Value": 3, "Wrap": true}),
        "Time" => json!({"Is24Hour": true, "Hour": 12, "Minute": 0}),
        "Date" => json!({"Year": 2026, "Month": 10, "Day": 24}),
        "Panel" => json!({"Color": "#E4E8EE"}),
        _ => json!({}),
    }
}

pub fn prop_type(name: &str) -> Option<&'static str> {
    PROP_TYPES.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| *v)
}

pub static PROP_TYPES: &[(&str, &str)] = &[
    ("Text", "String"),
    ("Hint", "String"),
    ("TextSize", "Integer"),
    ("TextColor", "Integer"),
    ("Color", "Integer"),
    ("Enabled", "Boolean"),
    ("Visible", "Boolean"),
    ("Checked", "Boolean"),
    ("Progress", "Integer"),
    ("Max", "Integer"),
    ("Rating", "Integer"),
    ("Stars", "Integer"),
    ("Format", "String"),
    ("TextOn", "String"),
    ("TextOff", "String"),
    ("MinValue", "Integer"),
    ("MaxValue", "Integer"),
    ("Value", "Integer"),
    ("Wrap", "Boolean"),
    ("Hour", "Integer"),
    ("Minute", "Integer"),
    ("Is24Hour", "Boolean"),
    ("Year", "Integer"),
    ("Month", "Integer"),
    ("Day", "Integer"),
];

#[derive(Clone, Copy)]
pub struct MethodRef {
    pub owner: &'static str,
    pub name: &'static str,
    pub ret: &'static str,
    pub params: &'static [&'static str],
    pub mode: &'static str,
}

pub static SETTERS: &[(&str, MethodRef)] = &[
    ("Text", MethodRef { owner: "Landroid/widget/TextView;", name: "setText", ret: "V", params: &["Ljava/lang/CharSequence;"], mode: "string" }),
    ("Hint", MethodRef { owner: "Landroid/widget/TextView;", name: "setHint", ret: "V", params: &["Ljava/lang/CharSequence;"], mode: "string" }),
    ("TextSize", MethodRef { owner: "Landroid/widget/TextView;", name: "setTextSize", ret: "V", params: &["F"], mode: "float" }),
    ("TextColor", MethodRef { owner: "Landroid/widget/TextView;", name: "setTextColor", ret: "V", params: &["I"], mode: "int" }),
    ("Color", MethodRef { owner: "Landroid/view/View;", name: "setBackgroundColor", ret: "V", params: &["I"], mode: "int" }),
    ("Enabled", MethodRef { owner: "Landroid/view/View;", name: "setEnabled", ret: "V", params: &["Z"], mode: "bool" }),
    ("Visible", MethodRef { owner: "Landroid/view/View;", name: "setVisibility", ret: "V", params: &["I"], mode: "vis" }),
    ("Checked", MethodRef { owner: "Landroid/widget/CompoundButton;", name: "setChecked", ret: "V", params: &["Z"], mode: "bool" }),
    ("Progress", MethodRef { owner: "Landroid/widget/ProgressBar;", name: "setProgress", ret: "V", params: &["I"], mode: "int" }),
    ("Max", MethodRef { owner: "Landroid/widget/ProgressBar;", name: "setMax", ret: "V", params: &["I"], mode: "int" }),
    ("Rating", MethodRef { owner: "Landroid/widget/RatingBar;", name: "setRating", ret: "V", params: &["F"], mode: "float" }),
    ("Stars", MethodRef { owner: "Landroid/widget/RatingBar;", name: "setNumStars", ret: "V", params: &["I"], mode: "int" }),
    ("Format", MethodRef { owner: "Landroid/widget/TextClock;", name: "setFormat12Hour", ret: "V", params: &["Ljava/lang/CharSequence;"], mode: "clock" }),
    ("TextOn", MethodRef { owner: "Landroid/widget/ToggleButton;", name: "setTextOn", ret: "V", params: &["Ljava/lang/CharSequence;"], mode: "string" }),
    ("TextOff", MethodRef { owner: "Landroid/widget/ToggleButton;", name: "setTextOff", ret: "V", params: &["Ljava/lang/CharSequence;"], mode: "string" }),
    ("MinValue", MethodRef { owner: "Landroid/widget/NumberPicker;", name: "setMinValue", ret: "V", params: &["I"], mode: "int" }),
    ("MaxValue", MethodRef { owner: "Landroid/widget/NumberPicker;", name: "setMaxValue", ret: "V", params: &["I"], mode: "int" }),
    ("Value", MethodRef { owner: "Landroid/widget/NumberPicker;", name: "setValue", ret: "V", params: &["I"], mode: "int" }),
    ("Wrap", MethodRef { owner: "Landroid/widget/NumberPicker;", name: "setWrapSelectorWheel", ret: "V", params: &["Z"], mode: "bool" }),
    ("Hour", MethodRef { owner: "Landroid/widget/TimePicker;", name: "setHour", ret: "V", params: &["I"], mode: "int" }),
    ("Minute", MethodRef { owner: "Landroid/widget/TimePicker;", name: "setMinute", ret: "V", params: &["I"], mode: "int" }),
    ("Is24Hour", MethodRef { owner: "Landroid/widget/TimePicker;", name: "setIs24HourView", ret: "V", params: &["Ljava/lang/Boolean;"], mode: "boxbool" }),
    ("Year", MethodRef { owner: "Landroid/widget/DatePicker;", name: "updateDate", ret: "V", params: &["I", "I", "I"], mode: "date-year" }),
    ("Month", MethodRef { owner: "Landroid/widget/DatePicker;", name: "updateDate", ret: "V", params: &["I", "I", "I"], mode: "date-month" }),
    ("Day", MethodRef { owner: "Landroid/widget/DatePicker;", name: "updateDate", ret: "V", params: &["I", "I", "I"], mode: "date-day" }),
];

pub fn setter(name: &str) -> Option<&'static MethodRef> {
    SETTERS.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v)
}

pub static GETTERS: &[(&str, MethodRef)] = &[
    ("Text", MethodRef { owner: "Landroid/widget/TextView;", name: "getText", ret: "Ljava/lang/CharSequence;", params: &[], mode: "text" }),
    ("Hint", MethodRef { owner: "Landroid/widget/TextView;", name: "getHint", ret: "Ljava/lang/CharSequence;", params: &[], mode: "text" }),
    ("Checked", MethodRef { owner: "Landroid/widget/CompoundButton;", name: "isChecked", ret: "Z", params: &[], mode: "bool" }),
    ("Enabled", MethodRef { owner: "Landroid/view/View;", name: "isEnabled", ret: "Z", params: &[], mode: "bool" }),
    ("Visible", MethodRef { owner: "Landroid/view/View;", name: "getVisibility", ret: "I", params: &[], mode: "vis" }),
    ("Progress", MethodRef { owner: "Landroid/widget/ProgressBar;", name: "getProgress", ret: "I", params: &[], mode: "int" }),
    ("Max", MethodRef { owner: "Landroid/widget/ProgressBar;", name: "getMax", ret: "I", params: &[], mode: "int" }),
    ("Rating", MethodRef { owner: "Landroid/widget/RatingBar;", name: "getRating", ret: "F", params: &[], mode: "float" }),
    ("Stars", MethodRef { owner: "Landroid/widget/RatingBar;", name: "getNumStars", ret: "I", params: &[], mode: "int" }),
    ("Format", MethodRef { owner: "Landroid/widget/TextClock;", name: "getFormat24Hour", ret: "Ljava/lang/CharSequence;", params: &[], mode: "text" }),
    ("TextOn", MethodRef { owner: "Landroid/widget/ToggleButton;", name: "getTextOn", ret: "Ljava/lang/CharSequence;", params: &[], mode: "text" }),
    ("TextOff", MethodRef { owner: "Landroid/widget/ToggleButton;", name: "getTextOff", ret: "Ljava/lang/CharSequence;", params: &[], mode: "text" }),
    ("MinValue", MethodRef { owner: "Landroid/widget/NumberPicker;", name: "getMinValue", ret: "I", params: &[], mode: "int" }),
    ("MaxValue", MethodRef { owner: "Landroid/widget/NumberPicker;", name: "getMaxValue", ret: "I", params: &[], mode: "int" }),
    ("Value", MethodRef { owner: "Landroid/widget/NumberPicker;", name: "getValue", ret: "I", params: &[], mode: "int" }),
    ("Wrap", MethodRef { owner: "Landroid/widget/NumberPicker;", name: "getWrapSelectorWheel", ret: "Z", params: &[], mode: "bool" }),
    ("Hour", MethodRef { owner: "Landroid/widget/TimePicker;", name: "getHour", ret: "I", params: &[], mode: "int" }),
    ("Minute", MethodRef { owner: "Landroid/widget/TimePicker;", name: "getMinute", ret: "I", params: &[], mode: "int" }),
    ("Is24Hour", MethodRef { owner: "Landroid/widget/TimePicker;", name: "is24HourView", ret: "Z", params: &[], mode: "bool" }),
    ("Year", MethodRef { owner: "Landroid/widget/DatePicker;", name: "getYear", ret: "I", params: &[], mode: "int" }),
    ("Month", MethodRef { owner: "Landroid/widget/DatePicker;", name: "getMonth", ret: "I", params: &[], mode: "month" }),
    ("Day", MethodRef { owner: "Landroid/widget/DatePicker;", name: "getDayOfMonth", ret: "I", params: &[], mode: "int" }),
];

pub fn getter(name: &str) -> Option<&'static MethodRef> {
    GETTERS.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v)
}

pub static FORM_EVENTS: &[&str] = &["OnCreate"];

pub fn blank_form(name: &str, title: Option<&str>) -> Value {
    json!({
        "name": name,
        "title": title.unwrap_or(name),
        "background": "#F7F8FA",
        "width": 360,
        "height": 640,
        "events": {},
        "components": [],
    })
}

pub fn new_component(kind: &str, name: &str, left: i32, top: i32) -> Value {
    let spec = widget(kind).unwrap_or_else(|| panic!("unknown component {kind}"));
    json!({
        "type": kind,
        "name": name,
        "left": left,
        "top": top,
        "width": spec.width,
        "height": spec.height,
        "props": default_props(kind),
        "events": {},
    })
}

pub fn parse_color(value: &Value) -> Result<u32, String> {
    if let Some(n) = value.as_i64() { return Ok((n as u32) & 0xFFFF_FFFF); }
    if let Some(n) = value.as_u64() { return Ok((n as u32) & 0xFFFF_FFFF); }
    let raw = value.as_str().unwrap_or("").trim().to_string();
    let text = raw.strip_prefix('#').or_else(|| raw.strip_prefix('$')).unwrap_or(&raw);
    let text = if text.len() == 3 { text.chars().flat_map(|c| [c, c]).collect::<String>() } else { text.to_string() };
    let text = if text.len() == 6 { format!("FF{text}") } else { text };
    if text.len() != 8 { return Err(format!("bad color {raw:?}")); }
    u32::from_str_radix(&text, 16).map_err(|_| format!("bad color {raw:?}"))
}
