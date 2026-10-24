"""Binary Android XML manifest writer. No aapt, no resources.arsc.

Attribute names occupy the front of the string pool so the resource map
lines up with them. Attributes on an element are sorted by (namespace,
resource id), with the null namespace last, which is what PackageManager expects.
"""

import struct

ANDROID_NS = "http://schemas.android.com/apk/res/android"
RES_NULL = 0xFFFFFFFF

# name -> android.R.attr id. These strings are pool entries 0..n-1.
ATTRS = [
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
]

TYPE_REFERENCE = 0x01
TYPE_STRING = 0x03
TYPE_INT = 0x10
TYPE_INT_BOOLEAN = 0x12

# Theme.DeviceDefault.Light.NoActionBar (API 14+)
THEME_DEVICE_DEFAULT_LIGHT_NO_ACTION_BAR = 16974124


def _utf16(text):
    raw = text.encode("utf-16-le")
    if len(text) >= 0x8000:
        raise ValueError("string too long for the manifest pool")
    return struct.pack("<H", len(text)) + raw + b"\x00\x00"


def _pad(blob):
    if len(blob) % 4:
        blob += b"\x00" * (4 - len(blob) % 4)
    return blob


class Strings:
    def __init__(self):
        self.items = []
        self.index = {}

    def add(self, text):
        if text not in self.index:
            self.index[text] = len(self.items)
            self.items.append(text)
        return self.index[text]

    def build(self):
        encoded = [_utf16(s) for s in self.items]
        header = 28
        offsets = []
        cursor = 0
        body = bytearray()
        for blob in encoded:
            offsets.append(cursor)
            body += blob
            cursor += len(blob)
        body = _pad(body)
        strings_start = header + 4 * len(self.items)
        size = strings_start + len(body)
        out = bytearray(size)
        struct.pack_into("<HHI", out, 0, 0x0001, 28, size)
        struct.pack_into("<IIIII", out, 8, len(self.items), 0, 0, strings_start, 0)
        for i, off in enumerate(offsets):
            struct.pack_into("<I", out, 28 + 4 * i, off)
        out[strings_start:] = body
        return bytes(out)


def _chunk(kind, body, header_size=16):
    size = header_size + len(body)
    head = struct.pack("<HHI", kind, header_size, size)
    # header_size includes the 8-byte chunk header. Extra header bytes are
    # already inside `body` only when header_size == 8. Callers pass the
    # bytes that follow the 8-byte chunk header, and header_size is 8 or 16.
    if header_size == 8:
        return head + body
    extra = body[: header_size - 8]
    rest = body[header_size - 8 :]
    return head + extra + rest


def _node(kind, body):
    # 8 byte chunk header + line + comment, then body.
    payload = struct.pack("<II", 1, RES_NULL) + body
    size = 8 + len(payload)
    return struct.pack("<HHI", kind, 16, size) + payload


def _ns(kind, prefix, uri):
    return _node(kind, struct.pack("<II", prefix, uri))


def _element(ns, name, attrs):
    # attrExt is 20 bytes: ns, name, attributeStart, attributeSize, count, id, class, style
    count = len(attrs)
    ext = struct.pack("<IIHHHHHH", ns, name, 20, 20, count, 0, 0, 0)
    raw = bytearray()
    for attr in attrs:
        raw += struct.pack(
            "<IIIHBBI",
            attr["ns"],
            attr["name"],
            attr["raw"],
            8,  # Res_value size
            0,
            attr["type"],
            attr["data"] & 0xFFFFFFFF,
        )
    return _node(0x0102, ext + raw)


def _end(ns, name):
    return _node(0x0103, struct.pack("<II", ns, name))


def build_manifest(project, forms):
    """project: name, package, title, versionCode, versionName, mainForm, minSdk, targetSdk, permissions.
    forms: list of activity class names (dotted), main first among those with a filter.
    """
    pool = Strings()
    attr_index = {}
    for name, _resid in ATTRS:
        attr_index[name] = pool.add(name)
    resmap = b"".join(struct.pack("<I", resid) for _name, resid in ATTRS)
    resmap_chunk = struct.pack("<HHI", 0x0180, 8, 8 + len(resmap)) + resmap

    prefix = pool.add("android")
    uri = pool.add(ANDROID_NS)
    package = project["package"]
    title = project.get("title") or project["name"]
    main = project["mainForm"]

    def S(text):
        return pool.add(str(text))

    def android_attr(attr, kind, data, raw=None):
        return {
            "ns": uri,
            "name": attr_index[attr],
            "raw": RES_NULL if raw is None else raw,
            "type": kind,
            "data": data,
            "resid": dict(ATTRS)[attr],
        }

    def plain_attr(attr_name, text):
        idx = S(text)
        return {
            "ns": RES_NULL,
            "name": S(attr_name),
            "raw": idx,
            "type": TYPE_STRING,
            "data": idx,
            "resid": RES_NULL,
        }

    def sort_attrs(attrs):
        def key(attr):
            ns_rank = 1 if attr["ns"] == RES_NULL else 0
            return (ns_rank, attr["resid"], attr["name"])

        return sorted(attrs, key=key)

    manifest_name = S("manifest")
    uses_sdk = S("uses-sdk")
    uses_perm = S("uses-permission")
    application = S("application")
    activity = S("activity")
    intent_filter = S("intent-filter")
    action = S("action")
    category = S("category")

    ver_name = S(str(project.get("versionName") or "1.0"))
    codename = S("15")
    label = S(title)

    manifest_attrs = sort_attrs([
        android_attr("versionCode", TYPE_INT, int(project.get("versionCode") or 1)),
        android_attr("versionName", TYPE_STRING, ver_name, ver_name),
        android_attr("compileSdkVersion", TYPE_INT, 35),
        android_attr("compileSdkVersionCodename", TYPE_STRING, codename, codename),
        plain_attr("package", package),
    ])
    sdk_attrs = sort_attrs([
        android_attr("minSdkVersion", TYPE_INT, int(project.get("minSdk") or 24)),
        android_attr("targetSdkVersion", TYPE_INT, int(project.get("targetSdk") or 28)),
    ])
    app_attrs = sort_attrs([
        android_attr("theme", TYPE_REFERENCE, THEME_DEVICE_DEFAULT_LIGHT_NO_ACTION_BAR),
        android_attr("label", TYPE_STRING, label, label),
        android_attr("debuggable", TYPE_INT_BOOLEAN, 0xFFFFFFFF),
        android_attr("hardwareAccelerated", TYPE_INT_BOOLEAN, 0xFFFFFFFF),
        android_attr("extractNativeLibs", TYPE_INT_BOOLEAN, 0xFFFFFFFF),
    ])

    parts = [
        _ns(0x0100, prefix, uri),
        _element(RES_NULL, manifest_name, manifest_attrs),
        _element(RES_NULL, uses_sdk, sdk_attrs),
        _end(RES_NULL, uses_sdk),
    ]
    for perm in project.get("permissions") or []:
        perm_s = S(perm)
        parts.append(_element(RES_NULL, uses_perm, sort_attrs([
            android_attr("name", TYPE_STRING, perm_s, perm_s),
        ])))
        parts.append(_end(RES_NULL, uses_perm))
    parts.append(_element(RES_NULL, application, app_attrs))
    for form in forms:
        dotted = package + "." + form
        form_s = S(dotted)
        label_s = S(form if form != main else title)
        exported = 0xFFFFFFFF if form == main else 0
        parts.append(_element(RES_NULL, activity, sort_attrs([
            android_attr("label", TYPE_STRING, label_s, label_s),
            android_attr("name", TYPE_STRING, form_s, form_s),
            android_attr("exported", TYPE_INT_BOOLEAN, exported),
        ])))
        if form == main:
            main_act = S("android.intent.action.MAIN")
            launcher = S("android.intent.category.LAUNCHER")
            parts.append(_element(RES_NULL, intent_filter, []))
            parts.append(_element(RES_NULL, action, sort_attrs([
                android_attr("name", TYPE_STRING, main_act, main_act),
            ])))
            parts.append(_end(RES_NULL, action))
            parts.append(_element(RES_NULL, category, sort_attrs([
                android_attr("name", TYPE_STRING, launcher, launcher),
            ])))
            parts.append(_end(RES_NULL, category))
            parts.append(_end(RES_NULL, intent_filter))
        parts.append(_end(RES_NULL, activity))
    parts.append(_end(RES_NULL, application))
    parts.append(_end(RES_NULL, manifest_name))
    parts.append(_ns(0x0101, prefix, uri))

    string_chunk = pool.build()
    body = string_chunk + resmap_chunk + b"".join(parts)
    size = 8 + len(body)
    return struct.pack("<HHI", 0x0003, 8, size) + body
