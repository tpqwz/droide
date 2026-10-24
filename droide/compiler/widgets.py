"""Native Android widget catalog used by the designer and the compiler."""

WIDGETS = {
    "Label": {
        "android": "android.widget.TextView",
        "desc": "Landroid/widget/TextView;",
        "group": "Text",
        "defaults": {"width": 200, "height": 36, "props": {"Text": "Label", "TextSize": 18, "TextColor": "#1A1A1A"}},
        "props": ["Text", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Clock": {
        "android": "android.widget.TextClock",
        "desc": "Landroid/widget/TextClock;",
        "group": "Text",
        "defaults": {"width": 200, "height": 36, "props": {"Format": "HH:mm", "TextSize": 18, "TextColor": "#1A1A1A"}},
        "props": ["Format", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Button": {
        "android": "android.widget.Button",
        "desc": "Landroid/widget/Button;",
        "group": "Buttons",
        "defaults": {"width": 160, "height": 44, "props": {"Text": "Button", "TextSize": 15}},
        "props": ["Text", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Toggle": {
        "android": "android.widget.ToggleButton",
        "desc": "Landroid/widget/ToggleButton;",
        "group": "Buttons",
        "defaults": {
            "width": 120,
            "height": 44,
            "props": {"TextOn": "On", "TextOff": "Off", "Checked": False, "TextSize": 15},
        },
        "props": ["TextOn", "TextOff", "Checked", "TextSize", "TextColor", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Edit": {
        "android": "android.widget.EditText",
        "desc": "Landroid/widget/EditText;",
        "group": "Input",
        "defaults": {"width": 240, "height": 44, "props": {"Hint": "Type here", "Text": "", "TextSize": 16}},
        "props": ["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Memo": {
        "android": "android.widget.EditText",
        "desc": "Landroid/widget/EditText;",
        "group": "Input",
        "defaults": {"width": 240, "height": 96, "props": {"Hint": "Notes", "Text": "", "TextSize": 16}},
        "props": ["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Number": {
        "android": "android.widget.EditText",
        "desc": "Landroid/widget/EditText;",
        "group": "Input",
        "defaults": {"width": 160, "height": 44, "props": {"Hint": "0", "Text": "", "TextSize": 16}},
        "props": ["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Password": {
        "android": "android.widget.EditText",
        "desc": "Landroid/widget/EditText;",
        "group": "Input",
        "defaults": {"width": 240, "height": 44, "props": {"Hint": "Password", "Text": "", "TextSize": 16}},
        "props": ["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Email": {
        "android": "android.widget.EditText",
        "desc": "Landroid/widget/EditText;",
        "group": "Input",
        "defaults": {"width": 240, "height": 44, "props": {"Hint": "name@example.com", "Text": "", "TextSize": 16}},
        "props": ["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Phone": {
        "android": "android.widget.EditText",
        "desc": "Landroid/widget/EditText;",
        "group": "Input",
        "defaults": {"width": 200, "height": 44, "props": {"Hint": "Phone", "Text": "", "TextSize": 16}},
        "props": ["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Check": {
        "android": "android.widget.CheckBox",
        "desc": "Landroid/widget/CheckBox;",
        "group": "Input",
        "defaults": {"width": 180, "height": 36, "props": {"Text": "Check", "Checked": False, "TextSize": 15}},
        "props": ["Text", "Checked", "TextSize", "TextColor", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Radio": {
        "android": "android.widget.RadioButton",
        "desc": "Landroid/widget/RadioButton;",
        "group": "Input",
        "defaults": {"width": 180, "height": 36, "props": {"Text": "Radio", "Checked": False, "TextSize": 15}},
        "props": ["Text", "Checked", "TextSize", "TextColor", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Switch": {
        "android": "android.widget.Switch",
        "desc": "Landroid/widget/Switch;",
        "group": "Input",
        "defaults": {"width": 180, "height": 36, "props": {"Text": "Switch", "Checked": False, "TextSize": 15}},
        "props": ["Text", "Checked", "TextSize", "TextColor", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Seek": {
        "android": "android.widget.SeekBar",
        "desc": "Landroid/widget/SeekBar;",
        "group": "Range",
        "defaults": {"width": 240, "height": 36, "props": {"Max": 100, "Progress": 40}},
        "props": ["Max", "Progress", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Progress": {
        "android": "android.widget.ProgressBar",
        "desc": "Landroid/widget/ProgressBar;",
        "group": "Range",
        "defaults": {"width": 240, "height": 24, "props": {"Max": 100, "Progress": 40}},
        "props": ["Max", "Progress", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Busy": {
        "android": "android.widget.ProgressBar",
        "desc": "Landroid/widget/ProgressBar;",
        "group": "Range",
        "defaults": {"width": 48, "height": 48, "props": {}},
        "props": ["Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Rating": {
        "android": "android.widget.RatingBar",
        "desc": "Landroid/widget/RatingBar;",
        "group": "Range",
        "defaults": {"width": 200, "height": 40, "props": {"Stars": 5, "Rating": 3}},
        "props": ["Stars", "Rating", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Picker": {
        "android": "android.widget.NumberPicker",
        "desc": "Landroid/widget/NumberPicker;",
        "group": "Pickers",
        "defaults": {
            "width": 120,
            "height": 160,
            "props": {"MinValue": 0, "MaxValue": 10, "Value": 3, "Wrap": True},
        },
        "props": ["MinValue", "MaxValue", "Value", "Wrap", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Time": {
        "android": "android.widget.TimePicker",
        "desc": "Landroid/widget/TimePicker;",
        "group": "Pickers",
        "defaults": {
            "width": 240,
            "height": 180,
            "props": {"Is24Hour": True, "Hour": 12, "Minute": 0},
        },
        "props": ["Is24Hour", "Hour", "Minute", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Date": {
        "android": "android.widget.DatePicker",
        "desc": "Landroid/widget/DatePicker;",
        "group": "Pickers",
        "defaults": {
            "width": 320,
            "height": 280,
            "props": {"Year": 2026, "Month": 10, "Day": 24},
        },
        "props": ["Year", "Month", "Day", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
    "Panel": {
        "android": "android.view.View",
        "desc": "Landroid/view/View;",
        "group": "Layout",
        "defaults": {"width": 200, "height": 80, "props": {"Color": "#E4E8EE"}},
        "props": ["Color", "Enabled", "Visible"],
        "events": ["OnClick"],
    },
}

PROP_TYPES = {
    "Text": "String",
    "Hint": "String",
    "TextSize": "Integer",
    "TextColor": "Integer",
    "Color": "Integer",
    "Enabled": "Boolean",
    "Visible": "Boolean",
    "Checked": "Boolean",
    "Progress": "Integer",
    "Max": "Integer",
    "Rating": "Integer",
    "Stars": "Integer",
    "Format": "String",
    "TextOn": "String",
    "TextOff": "String",
    "MinValue": "Integer",
    "MaxValue": "Integer",
    "Value": "Integer",
    "Wrap": "Boolean",
    "Hour": "Integer",
    "Minute": "Integer",
    "Is24Hour": "Boolean",
    "Year": "Integer",
    "Month": "Integer",
    "Day": "Integer",
}

# (owner class descriptor, method, return, parameter descriptors, mode)
SETTERS = {
    "Text": ("Landroid/widget/TextView;", "setText", "V", ("Ljava/lang/CharSequence;",), "string"),
    "Hint": ("Landroid/widget/TextView;", "setHint", "V", ("Ljava/lang/CharSequence;",), "string"),
    "TextSize": ("Landroid/widget/TextView;", "setTextSize", "V", ("F",), "float"),
    "TextColor": ("Landroid/widget/TextView;", "setTextColor", "V", ("I",), "int"),
    "Color": ("Landroid/view/View;", "setBackgroundColor", "V", ("I",), "int"),
    "Enabled": ("Landroid/view/View;", "setEnabled", "V", ("Z",), "bool"),
    "Visible": ("Landroid/view/View;", "setVisibility", "V", ("I",), "vis"),
    "Checked": ("Landroid/widget/CompoundButton;", "setChecked", "V", ("Z",), "bool"),
    "Progress": ("Landroid/widget/ProgressBar;", "setProgress", "V", ("I",), "int"),
    "Max": ("Landroid/widget/ProgressBar;", "setMax", "V", ("I",), "int"),
    "Rating": ("Landroid/widget/RatingBar;", "setRating", "V", ("F",), "float"),
    "Stars": ("Landroid/widget/RatingBar;", "setNumStars", "V", ("I",), "int"),
    # Both hour formats are set from this one string. See emit_setter mode "clock".
    "Format": ("Landroid/widget/TextClock;", "setFormat12Hour", "V", ("Ljava/lang/CharSequence;",), "clock"),
    "TextOn": ("Landroid/widget/ToggleButton;", "setTextOn", "V", ("Ljava/lang/CharSequence;",), "string"),
    "TextOff": ("Landroid/widget/ToggleButton;", "setTextOff", "V", ("Ljava/lang/CharSequence;",), "string"),
    "MinValue": ("Landroid/widget/NumberPicker;", "setMinValue", "V", ("I",), "int"),
    "MaxValue": ("Landroid/widget/NumberPicker;", "setMaxValue", "V", ("I",), "int"),
    "Value": ("Landroid/widget/NumberPicker;", "setValue", "V", ("I",), "int"),
    "Wrap": ("Landroid/widget/NumberPicker;", "setWrapSelectorWheel", "V", ("Z",), "bool"),
    "Hour": ("Landroid/widget/TimePicker;", "setHour", "V", ("I",), "int"),
    "Minute": ("Landroid/widget/TimePicker;", "setMinute", "V", ("I",), "int"),
    # setIs24HourView takes a Boolean object, not a primitive. See mode "boxbool".
    "Is24Hour": ("Landroid/widget/TimePicker;", "setIs24HourView", "V", ("Ljava/lang/Boolean;",), "boxbool"),
    # One updateDate call writes all three. Month in Droide is 1–12. See mode "date-*".
    "Year": ("Landroid/widget/DatePicker;", "updateDate", "V", ("I", "I", "I"), "date-year"),
    "Month": ("Landroid/widget/DatePicker;", "updateDate", "V", ("I", "I", "I"), "date-month"),
    "Day": ("Landroid/widget/DatePicker;", "updateDate", "V", ("I", "I", "I"), "date-day"),
}

GETTERS = {
    "Text": ("Landroid/widget/TextView;", "getText", "Ljava/lang/CharSequence;", (), "text"),
    "Hint": ("Landroid/widget/TextView;", "getHint", "Ljava/lang/CharSequence;", (), "text"),
    "Checked": ("Landroid/widget/CompoundButton;", "isChecked", "Z", (), "bool"),
    "Enabled": ("Landroid/view/View;", "isEnabled", "Z", (), "bool"),
    "Visible": ("Landroid/view/View;", "getVisibility", "I", (), "vis"),
    "Progress": ("Landroid/widget/ProgressBar;", "getProgress", "I", (), "int"),
    "Max": ("Landroid/widget/ProgressBar;", "getMax", "I", (), "int"),
    "Rating": ("Landroid/widget/RatingBar;", "getRating", "F", (), "float"),
    "Stars": ("Landroid/widget/RatingBar;", "getNumStars", "I", (), "int"),
    "Format": ("Landroid/widget/TextClock;", "getFormat24Hour", "Ljava/lang/CharSequence;", (), "text"),
    "TextOn": ("Landroid/widget/ToggleButton;", "getTextOn", "Ljava/lang/CharSequence;", (), "text"),
    "TextOff": ("Landroid/widget/ToggleButton;", "getTextOff", "Ljava/lang/CharSequence;", (), "text"),
    "MinValue": ("Landroid/widget/NumberPicker;", "getMinValue", "I", (), "int"),
    "MaxValue": ("Landroid/widget/NumberPicker;", "getMaxValue", "I", (), "int"),
    "Value": ("Landroid/widget/NumberPicker;", "getValue", "I", (), "int"),
    "Wrap": ("Landroid/widget/NumberPicker;", "getWrapSelectorWheel", "Z", (), "bool"),
    "Hour": ("Landroid/widget/TimePicker;", "getHour", "I", (), "int"),
    "Minute": ("Landroid/widget/TimePicker;", "getMinute", "I", (), "int"),
    "Is24Hour": ("Landroid/widget/TimePicker;", "is24HourView", "Z", (), "bool"),
    "Year": ("Landroid/widget/DatePicker;", "getYear", "I", (), "int"),
    # getMonth is 0–11. Mode "month" adds 1 so Droide sees calendar months.
    "Month": ("Landroid/widget/DatePicker;", "getMonth", "I", (), "month"),
    "Day": ("Landroid/widget/DatePicker;", "getDayOfMonth", "I", (), "int"),
}

FORM_EVENTS = ["OnCreate"]


def blank_form(name, title=None):
    return {
        "name": name,
        "title": title or name,
        "background": "#F7F8FA",
        "width": 360,
        "height": 640,
        "events": {},
        "components": [],
    }


def new_component(kind, name, left, top):
    spec = WIDGETS[kind]
    d = spec["defaults"]
    return {
        "type": kind,
        "name": name,
        "left": int(left),
        "top": int(top),
        "width": d["width"],
        "height": d["height"],
        "props": dict(d["props"]),
        "events": {},
    }


def parse_color(value):
    if isinstance(value, int):
        return value & 0xFFFFFFFF
    text = str(value).strip()
    if text.startswith("#"):
        text = text[1:]
    elif text.startswith("$"):
        text = text[1:]
    if len(text) == 3:
        text = "".join(ch * 2 for ch in text)
    if len(text) == 6:
        text = "FF" + text
    if len(text) != 8:
        raise ValueError(f"bad color {value!r}")
    return int(text, 16) & 0xFFFFFFFF
