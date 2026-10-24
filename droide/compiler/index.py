"""Lookup and Ingest keep a document library.

Every document is stored in a SQLite FTS4 table. That works from API 24.
A non-empty lookup uses AppSearch when SDK_INT is at least 31 and the
app_search service exists. DroideRt.lookup does not name an AppSearch type.
DroideIndex is loaded only for that call, and DroideIndex$Cb is created only
inside DroideIndex.query. An empty query lists the library through FTS4,
because AppSearch rejects an empty string. A null result from AppSearch means
the session failed, so the same query runs against FTS4. A non-null empty
string is a successful search with no hits.

Plain text, Markdown, HTML, CSV, and JSON are reduced to words before they
are indexed. Newlines are folded so one document stays one list row.

A match reads offsets(), which lists every hit. FTS4 and FTS5 snippet()
keep a single window of at most 64 tokens, so utfAt and span walk the
offsets and keep each hit, on its own row, with the text around it.
AppSearch does the same from MatchInfo ranges and reads every page.
"""

from .dex import ACC_CONSTRUCTOR, ACC_PUBLIC, ACC_STATIC, Asm, ClassModel, MethodModel

DB = "Landroid/database/sqlite/SQLiteDatabase;"
CURSOR = "Landroid/database/Cursor;"
FILE = "Ljava/io/File;"
CONTEXT = "Landroid/content/Context;"
STRING = "Ljava/lang/String;"
BUILDER = "Ljava/lang/StringBuilder;"

MANAGER = "Landroid/app/appsearch/AppSearchManager;"
SEARCH_CONTEXT = "Landroid/app/appsearch/AppSearchManager$SearchContext;"
SEARCH_CONTEXT_BUILDER = "Landroid/app/appsearch/AppSearchManager$SearchContext$Builder;"
SESSION = "Landroid/app/appsearch/AppSearchSession;"
RESULT = "Landroid/app/appsearch/AppSearchResult;"
SCHEMA = "Landroid/app/appsearch/AppSearchSchema;"
SCHEMA_BUILDER = "Landroid/app/appsearch/AppSearchSchema$Builder;"
PROP = "Landroid/app/appsearch/AppSearchSchema$PropertyConfig;"
STRING_PROP = "Landroid/app/appsearch/AppSearchSchema$StringPropertyConfig;"
STRING_PROP_BUILDER = "Landroid/app/appsearch/AppSearchSchema$StringPropertyConfig$Builder;"
SET_SCHEMA = "Landroid/app/appsearch/SetSchemaRequest;"
SET_SCHEMA_BUILDER = "Landroid/app/appsearch/SetSchemaRequest$Builder;"
PUT = "Landroid/app/appsearch/PutDocumentsRequest;"
PUT_BUILDER = "Landroid/app/appsearch/PutDocumentsRequest$Builder;"
DOC = "Landroid/app/appsearch/GenericDocument;"
DOC_BUILDER = "Landroid/app/appsearch/GenericDocument$Builder;"
SPEC = "Landroid/app/appsearch/SearchSpec;"
SPEC_BUILDER = "Landroid/app/appsearch/SearchSpec$Builder;"
HITS = "Landroid/app/appsearch/SearchResults;"
HIT = "Landroid/app/appsearch/SearchResult;"
MATCH = "Landroid/app/appsearch/SearchResult$MatchInfo;"
RANGE = "Landroid/app/appsearch/SearchResult$MatchRange;"
BATCH = "Landroid/app/appsearch/AppSearchBatchResult;"
BATCH_CB = "Landroid/app/appsearch/BatchResultCallback;"
CONSUMER = "Ljava/util/function/Consumer;"

# Characters of the column kept on each side of a hit. Both sides together
# are wider than the 64-token snippet() window, and every hit is kept.
SNIP_PAD = 256

SEEDS = (
    ("Ink", "txt", "Ink is a deep blue mixed on the swatch and kept in a glass jar."),
    ("Clay", "md", "# Clay\n\nClay is a warm red left on the shelf.\n\n- dry\n- fired"),
    ("Leaf", "html", "<h1>Leaf</h1><p>Leaf is the <b>green</b> of a fresh note.</p>"),
    ("Hours", "csv", "name,hour\nSand,pale gold\nPaper,evening"),
    ("Device", "json", '{"note":"Paper holds the private note from the device","where":"shelf"}'),
)


def _class_desc(package, name):
    return "L%s/%s;" % (package.replace(".", "/"), name)


def _m(owner, name, ret, params=()):
    return (owner, name, ret, tuple(params))


def _add(cls, name, ret, params, access, asm, registers, ins, outs):
    cls.method(MethodModel(name, ret, tuple(params), access, asm, registers, ins, outs))


def _append():
    return _m(BUILDER, "append", BUILDER, (STRING,))


def _length(owner):
    return _m(owner, "length", "I")


def _replace_all(asm, body, pattern, replacement):
    asm.const_string(0, pattern)
    asm.const_string(1, replacement)
    asm.invoke("virtual", _m(STRING, "replaceAll", STRING, (STRING, STRING)), [body, 0, 1])
    asm.move_result(body, True)


def _replace(asm, body, pattern, replacement):
    asm.const_string(0, pattern)
    asm.const_string(1, replacement)
    asm.invoke(
        "virtual",
        _m(STRING, "replace", STRING, ("Ljava/lang/CharSequence;", "Ljava/lang/CharSequence;")),
        [body, 0, 1],
    )
    asm.move_result(body, True)


def _jump_kind(asm, text, label):
    asm.const_string(1, text)
    asm.invoke("virtual", _m(STRING, "equals", "Z", ("Ljava/lang/Object;",)), [11, 1])
    asm.move_result(2, False)
    asm.if_zero("nez", 2, label)


def _fold(asm, body):
    _replace_all(asm, body, "\\s+", " ")
    asm.invoke("virtual", _m(STRING, "trim", STRING), [body])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _blank(asm, reg, label):
    asm.if_zero("nez", reg, label)
    asm.const_string(reg, "")
    asm.label(label)


def attach(dex, package, rt):
    rt_desc = _class_desc(package, "DroideRt")
    index_desc = _class_desc(package, "DroideIndex")
    cb_desc = _class_desc(package, "DroideIndex$Cb")

    database = Asm()
    _emit_db(database)
    _add(rt, "db", DB, (CONTEXT,), ACC_PUBLIC | ACC_STATIC, database, 8, 1, 2)

    ready = Asm()
    _emit_ready(ready)
    _add(rt, "ready", "V", (DB,), ACC_PUBLIC | ACC_STATIC, ready, 8, 1, 2)

    text = Asm()
    _emit_plain(text)
    _add(rt, "plain", STRING, (STRING, STRING), ACC_PUBLIC | ACC_STATIC, text, 12, 2, 3)

    row = Asm()
    _emit_add_row(row, rt_desc)
    _add(rt, "addRow", "V", (DB, STRING, STRING, STRING), ACC_PUBLIC | ACC_STATIC, row, 12, 4, 3)

    shelf = Asm()
    _emit_fill(shelf, rt_desc)
    _add(rt, "fill", "V", (DB,), ACC_PUBLIC | ACC_STATIC, shelf, 8, 1, 4)

    terms = Asm()
    _emit_terms(terms)
    _add(rt, "terms", STRING, (STRING,), ACC_PUBLIC | ACC_STATIC, terms, 12, 1, 3)

    place = Asm()
    _emit_utf_at(place)
    _add(rt, "utfAt", "I", (STRING, "I"), ACC_PUBLIC | ACC_STATIC, place, 8, 2, 2)

    window = Asm()
    _emit_span(window)
    _add(rt, "span", STRING, (STRING, "I", "I"), ACC_PUBLIC | ACC_STATIC, window, 12, 3, 3)

    clip = Asm()
    _emit_snip(clip, rt_desc)
    _add(rt, "snip", STRING, (STRING, STRING, "I"), ACC_PUBLIC | ACC_STATIC, clip, 16, 3, 3)

    poured = Asm()
    _emit_pour(poured)
    _add(rt, "pour", "V", (BUILDER, STRING, STRING, STRING), ACC_PUBLIC | ACC_STATIC, poured, 16, 4, 3)

    listed = Asm()
    _emit_rows(listed, rt_desc)
    _add(rt, "rows", STRING, (DB, STRING), ACC_PUBLIC | ACC_STATIC, listed, 16, 2, 4)

    ingest = Asm()
    _emit_ingest(ingest, rt_desc)
    _add(rt, "ingest", STRING, (CONTEXT, STRING, STRING, STRING), ACC_PUBLIC | ACC_STATIC, ingest, 16, 4, 4)

    lookup = Asm()
    _emit_lookup(lookup, rt_desc, index_desc)
    _add(rt, "lookup", STRING, (CONTEXT, STRING), ACC_PUBLIC | ACC_STATIC, lookup, 16, 2, 2)

    api = ClassModel(index_desc, "Ljava/lang/Object;", ACC_PUBLIC, source="DroideIndex")
    request = Asm()
    _emit_schema(request, index_desc)
    _add(api, "schema", SET_SCHEMA, (), ACC_PUBLIC | ACC_STATIC, request, 8, 0, 3)
    prop = Asm()
    _emit_prop(prop)
    _add(api, "prop", STRING_PROP, (STRING, "I"), ACC_PUBLIC | ACC_STATIC, prop, 8, 2, 2)
    one = Asm()
    _emit_add_doc(one)
    _add(
        api, "addDoc", "V",
        (PUT_BUILDER, STRING, STRING, STRING, STRING),
        ACC_PUBLIC | ACC_STATIC, one, 12, 5, 4,
    )
    docs = Asm()
    _emit_documents(docs, rt_desc, index_desc)
    _add(api, "documents", PUT, (CONTEXT,), ACC_PUBLIC | ACC_STATIC, docs, 16, 1, 5)
    spec = Asm()
    _emit_spec(spec)
    _add(api, "spec", SPEC, (), ACC_PUBLIC | ACC_STATIC, spec, 8, 0, 2)
    hit = Asm()
    _emit_line(hit, rt_desc)
    _add(api, "line", STRING, (HIT,), ACC_PUBLIC | ACC_STATIC, hit, 16, 1, 3)
    lines = Asm()
    _emit_lines(lines, index_desc)
    _add(api, "lines", STRING, ("Ljava/util/List;",), ACC_PUBLIC | ACC_STATIC, lines, 16, 1, 2)
    query = Asm()
    _emit_query(query, cb_desc)
    _add(api, "query", STRING, (CONTEXT, STRING), ACC_PUBLIC | ACC_STATIC, query, 16, 2, 4)
    dex.add(api)

    cb = ClassModel(
        cb_desc, "Ljava/lang/Object;", ACC_PUBLIC,
        interfaces=[CONSUMER, BATCH_CB], source="DroideIndex",
    )
    cb.field("latch", "Ljava/util/concurrent/CountDownLatch;")
    cb.field("executor", "Ljava/util/concurrent/Executor;")
    cb.field("query", STRING)
    cb.field("context", "Ljava/lang/Object;")
    cb.field("stage", "I")
    cb.field("session", "Ljava/lang/Object;")
    cb.field("hits", "Ljava/lang/Object;")
    cb.field("text", STRING)

    init = Asm()
    init.invoke("direct", _m("Ljava/lang/Object;", "<init>", "V"), [0])
    init.ret(0, "void")
    _add(cb, "<init>", "V", (), ACC_PUBLIC | ACC_CONSTRUCTOR, init, 1, 1, 1)

    finish = Asm()
    _emit_finish(finish, cb_desc)
    _add(cb, "finish", "V", (), ACC_PUBLIC, finish, 6, 1, 1)

    accept = Asm()
    _emit_accept(accept, index_desc, cb_desc)
    _add(cb, "accept", "V", ("Ljava/lang/Object;",), ACC_PUBLIC, accept, 16, 2, 5)

    on_result = Asm()
    _emit_on_result(on_result, index_desc, cb_desc)
    _add(cb, "onResult", "V", (BATCH,), ACC_PUBLIC, on_result, 12, 2, 3)

    on_error = Asm()
    _emit_on_error(on_error, cb_desc)
    _add(cb, "onSystemError", "V", ("Ljava/lang/Throwable;",), ACC_PUBLIC, on_error, 4, 2, 1)
    dex.add(cb)


def _emit_db(asm):
    asm.const_string(0, "library.db")
    asm.invoke("virtual", _m(CONTEXT, "getDatabasePath", FILE, (STRING,)), [7, 0])
    asm.move_result(0, True)
    asm.invoke("virtual", _m(FILE, "getParentFile", FILE), [0])
    asm.move_result(1, True)
    asm.invoke("virtual", _m(FILE, "mkdirs", "Z"), [1])
    asm.move_result(2, False)
    asm.invoke("virtual", _m(FILE, "getPath", STRING), [0])
    asm.move_result(0, True)
    asm.const(1, 0)
    asm.invoke(
        "static",
        _m(DB, "openOrCreateDatabase", DB, (STRING, "Landroid/database/sqlite/SQLiteDatabase$CursorFactory;")),
        [0, 1],
    )
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_ready(asm):
    asm.const_string(0, "CREATE VIRTUAL TABLE IF NOT EXISTS docs USING fts4(title, body, kind)")
    asm.invoke("virtual", _m(DB, "execSQL", "V", (STRING,)), [7, 0])
    asm.ret(0, "void")


def _emit_plain(asm):
    asm.if_zero("nez", 10, "plain_body")
    asm.const_string(10, "")
    asm.label("plain_body")
    asm.if_zero("nez", 11, "plain_kind")
    asm.const_string(11, "txt")
    asm.label("plain_kind")
    asm.new_instance(0, "Ljava/util/Locale;")
    asm.const_string(1, "en")
    asm.invoke("direct", _m("Ljava/util/Locale;", "<init>", "V", (STRING,)), [0, 1])
    asm.invoke(
        "virtual",
        _m(STRING, "toLowerCase", STRING, ("Ljava/util/Locale;",)),
        [11, 0],
    )
    asm.move_result(11, True)
    _jump_kind(asm, "html", "plain_html")
    _jump_kind(asm, "htm", "plain_html")
    _jump_kind(asm, "md", "plain_md")
    _jump_kind(asm, "markdown", "plain_md")
    _jump_kind(asm, "csv", "plain_csv")
    _jump_kind(asm, "json", "plain_json")
    _fold(asm, 10)

    asm.label("plain_html")
    _replace_all(asm, 10, "(?s)<[^>]*>", " ")
    _replace(asm, 10, "&amp;", "&")
    _replace(asm, 10, "&lt;", "<")
    _replace(asm, 10, "&gt;", ">")
    _replace(asm, 10, "&nbsp;", " ")
    _fold(asm, 10)

    asm.label("plain_md")
    _replace_all(asm, 10, "\\[([^\\]]+)\\]\\([^)]*\\)", "$1")
    _replace_all(asm, 10, "[#*_`>]+", " ")
    _fold(asm, 10)

    asm.label("plain_csv")
    _replace(asm, 10, ",", " ")
    _fold(asm, 10)

    asm.label("plain_json")
    _replace_all(asm, 10, "[\\{\\}\\[\\]\":,]", " ")
    _fold(asm, 10)


def _emit_add_row(asm, rt):
    asm.invoke("static", _m(rt, "plain", STRING, (STRING, STRING)), [10, 11])
    asm.move_result(0, True)
    asm.const_string(1, "INSERT INTO docs(title, body, kind) VALUES(?,?,?)")
    asm.const(2, 3)
    asm.new_array(3, 2, "[Ljava/lang/Object;")
    asm.const(2, 0)
    asm.aput_object(9, 3, 2)
    asm.const(2, 1)
    asm.aput_object(0, 3, 2)
    asm.const(2, 2)
    asm.aput_object(11, 3, 2)
    asm.invoke("virtual", _m(DB, "execSQL", "V", (STRING, "[Ljava/lang/Object;")), [8, 1, 3])
    asm.ret(0, "void")


def _emit_fill(asm, rt):
    asm.const_string(0, "SELECT count(*) FROM docs")
    asm.const(1, 0)
    asm.invoke("virtual", _m(DB, "rawQuery", CURSOR, (STRING, "[Ljava/lang/String;")), [7, 0, 1])
    asm.move_result(0, True)
    asm.invoke("interface", _m(CURSOR, "moveToNext", "Z"), [0])
    asm.move_result(1, False)
    asm.const(2, 0)
    asm.invoke("interface", _m(CURSOR, "getInt", "I", ("I",)), [0, 2])
    asm.move_result(1, False)
    asm.invoke("interface", _m(CURSOR, "close", "V"), [0])
    asm.if_zero("nez", 1, "fill_done")
    for title, kind, body in SEEDS:
        asm.const_string(0, title)
        asm.const_string(1, body)
        asm.const_string(2, kind)
        asm.invoke("static", _m(rt, "addRow", "V", (DB, STRING, STRING, STRING)), [7, 0, 1, 2])
    asm.ret(0, "void")
    asm.label("fill_done")
    asm.ret(0, "void")


def _emit_terms(asm):
    asm.if_zero("nez", 11, "terms_have")
    asm.const_string(0, "")
    asm.ret(0, "obj")
    asm.label("terms_have")
    asm.new_instance(0, "Ljava/util/Locale;")
    asm.const_string(1, "en")
    asm.invoke("direct", _m("Ljava/util/Locale;", "<init>", "V", (STRING,)), [0, 1])
    asm.invoke("virtual", _m(STRING, "toLowerCase", STRING, ("Ljava/util/Locale;",)), [11, 0])
    asm.move_result(11, True)
    _replace_all(asm, 11, "[^a-z0-9]+", " ")
    asm.invoke("virtual", _m(STRING, "trim", STRING), [11])
    asm.move_result(11, True)
    asm.invoke("virtual", _length(STRING), [11])
    asm.move_result(2, False)
    asm.if_zero("nez", 2, "terms_star")
    asm.const_string(0, "")
    asm.ret(0, "obj")
    asm.label("terms_star")
    _replace(asm, 11, " ", "* ")
    asm.const_string(0, "*")
    asm.invoke("virtual", _m(STRING, "concat", STRING, (STRING,)), [11, 0])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_utf_at(asm):
    asm.if_zero("nez", 6, "utf_text")
    asm.const(0, 0)
    asm.ret(0, "int")
    asm.label("utf_text")
    asm.invoke("virtual", _length(STRING), [6])
    asm.move_result(2, False)
    asm.const(0, 0)
    asm.const(1, 0)
    asm.const(3, 0)
    asm.const(4, 0)
    asm.label("utf_loop")
    asm.if_cmp("ge", 0, 2, "utf_done")
    asm.if_cmp("ge", 1, 7, "utf_done")
    asm.invoke("virtual", _m(STRING, "charAt", "C", ("I",)), [6, 0])
    asm.move_result(3, False)
    asm.const(4, 128)
    asm.if_cmp("lt", 3, 4, "utf_1")
    asm.const(4, 2048)
    asm.if_cmp("lt", 3, 4, "utf_2")
    asm.const(4, 0xD800)
    asm.if_cmp("lt", 3, 4, "utf_3")
    asm.const(4, 0xDC00)
    asm.if_cmp("ge", 3, 4, "utf_3")
    asm.const(4, 4)
    asm.binop(0x90, 1, 1, 4, "add-int")
    asm.const(4, 1)
    asm.binop(0x90, 0, 0, 4, "add-int")
    asm.if_cmp("ge", 0, 2, "utf_done")
    asm.goto("utf_advance")
    asm.label("utf_1")
    asm.const(4, 1)
    asm.goto("utf_add")
    asm.label("utf_2")
    asm.const(4, 2)
    asm.goto("utf_add")
    asm.label("utf_3")
    asm.const(4, 3)
    asm.label("utf_add")
    asm.binop(0x90, 1, 1, 4, "add-int")
    asm.label("utf_advance")
    asm.const(4, 1)
    asm.binop(0x90, 0, 0, 4, "add-int")
    asm.goto("utf_loop")
    asm.label("utf_done")
    asm.ret(0, "int")


def _push_reg(asm, reg):
    asm.invoke("virtual", _append(), [4, reg])
    asm.move_result(11, True)


def _emit_snip(asm, rt):
    asm.if_zero("nez", 13, "snip_text")
    asm.const_string(13, "")
    asm.label("snip_text")
    asm.if_zero("nez", 14, "snip_off")
    asm.const_string(14, "")
    asm.label("snip_off")
    asm.new_instance(4, BUILDER)
    asm.invoke("direct", _m(BUILDER, "<init>", "V"), [4])
    asm.const(0, 0)
    asm.const(1, 0)
    asm.const(2, 0)
    asm.const(3, 0)
    asm.const(5, 0)
    asm.const(7, 0)
    asm.const(8, 0)
    asm.const(9, 0)
    asm.const(10, 0)
    asm.const(12, 0)
    asm.const_string(6, "")
    asm.const_string(11, "")
    asm.label("snip_next")
    asm.invoke("virtual", _length(STRING), [14])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "snip_done")
    asm.const_string(6, " ")
    asm.invoke("virtual", _m(STRING, "indexOf", "I", (STRING,)), [14, 6])
    asm.move_result(0, False)
    asm.const(1, -1)
    asm.if_cmp("eq", 0, 1, "snip_last")
    asm.const(2, 0)
    asm.invoke("virtual", _m(STRING, "substring", STRING, ("I", "I")), [14, 2, 0])
    asm.move_result(6, True)
    asm.const(2, 1)
    asm.binop(0x90, 2, 0, 2, "add-int")
    asm.invoke("virtual", _length(STRING), [14])
    asm.move_result(1, False)
    asm.invoke("virtual", _m(STRING, "substring", STRING, ("I", "I")), [14, 2, 1])
    asm.move_result(14, True)
    asm.goto("snip_parse")
    asm.label("snip_last")
    asm.move(6, 14, True)
    asm.const_string(14, "")
    asm.label("snip_parse")
    asm.invoke("static", _m("Ljava/lang/Integer;", "parseInt", "I", (STRING,)), [6])
    asm.move_result(0, False)
    asm.const(1, 0)
    asm.if_cmp("ne", 3, 1, "snip_ph1")
    asm.move(7, 0, False)
    asm.goto("snip_step")
    asm.label("snip_ph1")
    asm.const(1, 1)
    asm.if_cmp("ne", 3, 1, "snip_ph2")
    asm.goto("snip_step")
    asm.label("snip_ph2")
    asm.const(1, 2)
    asm.if_cmp("ne", 3, 1, "snip_ph3")
    asm.move(8, 0, False)
    asm.goto("snip_step")
    asm.label("snip_ph3")
    asm.move(9, 0, False)
    asm.const(1, 0)
    asm.if_cmp("ne", 7, 15, "snip_step")
    asm.if_cmp("lt", 8, 1, "snip_step")
    asm.if_cmp("le", 9, 1, "snip_step")
    asm.binop(0x90, 2, 8, 9, "add-int")
    asm.invoke("static", _m(rt, "utfAt", "I", (STRING, "I")), [13, 8])
    asm.move_result(10, False)
    asm.invoke("static", _m(rt, "utfAt", "I", (STRING, "I")), [13, 2])
    asm.move_result(12, False)
    asm.if_cmp("ge", 10, 12, "snip_step")
    asm.invoke("static", _m(rt, "span", STRING, (STRING, "I", "I")), [13, 10, 12])
    asm.move_result(6, True)
    asm.if_zero("eqz", 6, "snip_step")
    asm.invoke("virtual", _length(STRING), [6])
    asm.move_result(5, False)
    asm.if_zero("eqz", 5, "snip_step")
    asm.invoke("virtual", _length(BUILDER), [4])
    asm.move_result(2, False)
    asm.if_zero("eqz", 2, "snip_add")
    asm.const_string(11, "\n")
    asm.invoke("virtual", _append(), [4, 11])
    asm.move_result(11, True)
    asm.label("snip_add")
    _push_reg(asm, 6)
    asm.goto("snip_step")
    asm.label("snip_step")
    asm.const(1, 3)
    asm.if_cmp("ne", 3, 1, "snip_inc")
    asm.const(3, 0)
    asm.goto("snip_next")
    asm.label("snip_inc")
    asm.const(1, 1)
    asm.binop(0x90, 3, 3, 1, "add-int")
    asm.goto("snip_next")
    asm.label("snip_done")
    asm.invoke("virtual", _m(BUILDER, "toString", STRING), [4])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_span(asm):
    # v9 text, v10 start, v11 end, in Java chars. One hit, marked.
    asm.if_zero("nez", 9, "span_text")
    asm.const_string(9, "")
    asm.label("span_text")
    asm.invoke("virtual", _length(STRING), [9])
    asm.move_result(5, False)
    asm.const(1, 0)
    asm.if_cmp("ge", 10, 1, "span_start")
    asm.const(10, 0)
    asm.label("span_start")
    asm.if_cmp("ge", 11, 10, "span_end0")
    asm.move(11, 10, False)
    asm.label("span_end0")
    asm.if_cmp("le", 11, 5, "span_end1")
    asm.move(11, 5, False)
    asm.label("span_end1")
    asm.if_cmp("lt", 10, 11, "span_hit")
    asm.const_string(0, "")
    asm.ret(0, "obj")
    asm.label("span_hit")
    asm.const(0, SNIP_PAD)
    asm.binop(0x91, 0, 10, 0, "sub-int")
    asm.const(1, 0)
    asm.if_cmp("ge", 0, 1, "span_ws")
    asm.const(0, 0)
    asm.label("span_ws")
    asm.const(1, SNIP_PAD)
    asm.binop(0x90, 1, 11, 1, "add-int")
    asm.if_cmp("le", 1, 5, "span_we")
    asm.move(1, 5, False)
    asm.label("span_we")
    asm.new_instance(4, BUILDER)
    asm.invoke("direct", _m(BUILDER, "<init>", "V"), [4])
    asm.const_string(2, "")
    asm.const_string(6, "")
    asm.const(3, 0)
    asm.if_cmp("le", 0, 3, "span_pre")
    asm.const_string(6, "…")
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(2, True)
    asm.label("span_pre")
    asm.if_cmp("ge", 0, 10, "span_mark")
    asm.invoke("virtual", _m(STRING, "substring", STRING, ("I", "I")), [9, 0, 10])
    asm.move_result(6, True)
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(2, True)
    asm.label("span_mark")
    asm.const_string(6, "«")
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(2, True)
    asm.invoke("virtual", _m(STRING, "substring", STRING, ("I", "I")), [9, 10, 11])
    asm.move_result(6, True)
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(2, True)
    asm.const_string(6, "»")
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(2, True)
    asm.if_cmp("ge", 11, 1, "span_tail")
    asm.invoke("virtual", _m(STRING, "substring", STRING, ("I", "I")), [9, 11, 1])
    asm.move_result(6, True)
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(2, True)
    asm.label("span_tail")
    asm.if_cmp("ge", 1, 5, "span_done")
    asm.const_string(6, "…")
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(2, True)
    asm.label("span_done")
    asm.invoke("virtual", _m(BUILDER, "toString", STRING), [4])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_pour(asm):
    # v12 result builder, v13 title, v14 kind, v15 snippets, one per line.
    asm.if_zero("nez", 15, "pour_clips")
    asm.ret(0, "void")
    asm.label("pour_clips")
    asm.invoke("virtual", _length(STRING), [15])
    asm.move_result(0, False)
    asm.if_zero("nez", 0, "pour_go")
    asm.ret(0, "void")
    asm.label("pour_go")
    asm.const(0, 0)
    asm.const(1, 0)
    asm.const(3, 0)
    asm.const(5, 0)
    asm.const_string(2, "")
    asm.const_string(6, "")
    asm.label("pour_next")
    asm.invoke("virtual", _length(STRING), [15])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "pour_done")
    asm.const_string(2, "\n")
    asm.invoke("virtual", _m(STRING, "indexOf", "I", (STRING,)), [15, 2])
    asm.move_result(0, False)
    asm.const(1, -1)
    asm.if_cmp("eq", 0, 1, "pour_last")
    asm.const(1, 0)
    asm.invoke("virtual", _m(STRING, "substring", STRING, ("I", "I")), [15, 1, 0])
    asm.move_result(6, True)
    asm.const(1, 1)
    asm.binop(0x90, 1, 0, 1, "add-int")
    asm.invoke("virtual", _length(STRING), [15])
    asm.move_result(3, False)
    asm.invoke("virtual", _m(STRING, "substring", STRING, ("I", "I")), [15, 1, 3])
    asm.move_result(15, True)
    asm.goto("pour_line")
    asm.label("pour_last")
    asm.move(6, 15, True)
    asm.const_string(15, "")
    asm.label("pour_line")
    asm.invoke("virtual", _length(STRING), [6])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "pour_next")
    asm.invoke("virtual", _length(BUILDER), [12])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "pour_add")
    asm.const_string(2, "\n")
    asm.invoke("virtual", _append(), [12, 2])
    asm.move_result(2, True)
    asm.label("pour_add")
    asm.invoke("virtual", _append(), [12, 13])
    asm.move_result(2, True)
    asm.const_string(2, "  ·  ")
    asm.invoke("virtual", _append(), [12, 2])
    asm.move_result(2, True)
    asm.invoke("virtual", _append(), [12, 14])
    asm.move_result(2, True)
    asm.const_string(2, "  —  ")
    asm.invoke("virtual", _append(), [12, 2])
    asm.move_result(2, True)
    asm.invoke("virtual", _append(), [12, 6])
    asm.move_result(2, True)
    asm.goto("pour_next")
    asm.label("pour_done")
    asm.ret(0, "void")


def _emit_rows(asm, rt):
    asm.new_instance(4, BUILDER)
    asm.invoke("direct", _m(BUILDER, "<init>", "V"), [4])
    asm.const_string(1, "\n")
    asm.const_string(2, "")
    asm.const(3, 0)
    asm.const(5, 0)
    asm.const_string(6, "")
    asm.invoke("virtual", _length(STRING), [15])
    asm.move_result(5, False)
    asm.if_zero("eqz", 5, "rows_all")
    asm.const_string(
        2,
        "SELECT title, body, kind, offsets(docs) FROM docs WHERE docs MATCH ? ORDER BY rowid",
    )
    asm.const(8, 1)
    asm.new_array(9, 8, "[Ljava/lang/String;")
    asm.const(8, 0)
    asm.aput_object(15, 9, 8)
    asm.invoke("virtual", _m(DB, "rawQuery", CURSOR, (STRING, "[Ljava/lang/String;")), [14, 2, 9])
    asm.move_result(0, True)
    asm.const(3, 0)
    asm.const(5, 0)
    asm.const_string(2, "")
    asm.const_string(6, "")
    asm.const_string(7, "")
    asm.const_string(8, "")
    asm.const_string(10, "")
    asm.const_string(11, "")
    asm.const_string(12, "")
    asm.label("rows_hit")
    asm.invoke("interface", _m(CURSOR, "moveToNext", "Z"), [0])
    asm.move_result(5, False)
    asm.if_zero("eqz", 5, "rows_hit_done")
    asm.const(3, 0)
    asm.invoke("interface", _m(CURSOR, "getString", STRING, ("I",)), [0, 3])
    asm.move_result(6, True)
    _blank(asm, 6, "rows_title")
    asm.const(3, 1)
    asm.invoke("interface", _m(CURSOR, "getString", STRING, ("I",)), [0, 3])
    asm.move_result(7, True)
    _blank(asm, 7, "rows_body_text")
    asm.const(3, 2)
    asm.invoke("interface", _m(CURSOR, "getString", STRING, ("I",)), [0, 3])
    asm.move_result(8, True)
    _blank(asm, 8, "rows_kind")
    asm.const(3, 3)
    asm.invoke("interface", _m(CURSOR, "getString", STRING, ("I",)), [0, 3])
    asm.move_result(12, True)
    _blank(asm, 12, "rows_offsets")
    asm.const(3, 0)
    asm.invoke("static", _m(rt, "snip", STRING, (STRING, STRING, "I")), [6, 12, 3])
    asm.move_result(10, True)
    asm.const(3, 1)
    asm.invoke("static", _m(rt, "snip", STRING, (STRING, STRING, "I")), [7, 12, 3])
    asm.move_result(11, True)
    asm.invoke("virtual", _length(STRING), [10])
    asm.move_result(3, False)
    asm.invoke("virtual", _length(STRING), [11])
    asm.move_result(5, False)
    asm.binop(0x90, 3, 3, 5, "add-int")
    asm.if_zero("nez", 3, "rows_clips")
    asm.invoke("virtual", _length(BUILDER), [4])
    asm.move_result(5, False)
    asm.if_zero("eqz", 5, "rows_plain")
    asm.invoke("virtual", _append(), [4, 1])
    asm.move_result(2, True)
    asm.label("rows_plain")
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(2, True)
    asm.const_string(2, "  ·  ")
    asm.invoke("virtual", _append(), [4, 2])
    asm.move_result(2, True)
    asm.invoke("virtual", _append(), [4, 8])
    asm.move_result(2, True)
    asm.goto("rows_hit")
    asm.label("rows_clips")
    asm.invoke("static", _m(rt, "pour", "V", (BUILDER, STRING, STRING, STRING)), [4, 6, 8, 10])
    asm.invoke("static", _m(rt, "pour", "V", (BUILDER, STRING, STRING, STRING)), [4, 6, 8, 11])
    asm.goto("rows_hit")
    asm.label("rows_hit_done")
    asm.invoke("interface", _m(CURSOR, "close", "V"), [0])
    asm.invoke("virtual", _m(BUILDER, "toString", STRING), [4])
    asm.move_result(0, True)
    asm.ret(0, "obj")
    asm.label("rows_all")
    asm.const_string(2, "SELECT title || '  ·  ' || kind FROM docs ORDER BY rowid")
    asm.const(9, 0)
    asm.invoke("virtual", _m(DB, "rawQuery", CURSOR, (STRING, "[Ljava/lang/String;")), [14, 2, 9])
    asm.move_result(0, True)
    asm.const(3, 0)
    asm.const(5, 0)
    asm.const(8, 0)
    asm.const(9, 0)
    asm.label("rows_scan")
    asm.invoke("interface", _m(CURSOR, "moveToNext", "Z"), [0])
    asm.move_result(5, False)
    asm.if_zero("eqz", 5, "rows_done")
    asm.const(3, 0)
    asm.invoke("interface", _m(CURSOR, "getString", STRING, ("I",)), [0, 3])
    asm.move_result(6, True)
    asm.if_zero("nez", 6, "rows_text")
    asm.const_string(6, "")
    asm.label("rows_text")
    asm.invoke("virtual", _length(BUILDER), [4])
    asm.move_result(5, False)
    asm.if_zero("eqz", 5, "rows_add")
    asm.invoke("virtual", _append(), [4, 1])
    asm.move_result(2, True)
    asm.label("rows_add")
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(2, True)
    asm.goto("rows_scan")
    asm.label("rows_done")
    asm.invoke("interface", _m(CURSOR, "close", "V"), [0])
    asm.invoke("virtual", _m(BUILDER, "toString", STRING), [4])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_ingest(asm, rt):
    asm.if_zero("eqz", 13, "ingest_title")
    asm.invoke("virtual", _length(STRING), [13])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "ingest_title")
    asm.if_zero("eqz", 14, "ingest_body")
    asm.invoke("virtual", _length(STRING), [14])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "ingest_body")
    asm.if_zero("eqz", 15, "ingest_kind")
    asm.invoke("virtual", _length(STRING), [15])
    asm.move_result(0, False)
    asm.if_zero("nez", 0, "ingest_open")
    asm.label("ingest_kind")
    asm.const_string(15, "txt")
    asm.label("ingest_open")
    asm.invoke("static", _m(rt, "db", DB, (CONTEXT,)), [12])
    asm.move_result(0, True)
    asm.invoke("static", _m(rt, "ready", "V", (DB,)), [0])
    asm.invoke("static", _m(rt, "addRow", "V", (DB, STRING, STRING, STRING)), [0, 13, 14, 15])
    asm.invoke("virtual", _m(DB, "close", "V"), [0])
    asm.new_instance(1, BUILDER)
    asm.invoke("direct", _m(BUILDER, "<init>", "V"), [1])
    asm.const_string(2, "Added ")
    asm.invoke("virtual", _append(), [1, 2])
    asm.move_result(3, True)
    asm.invoke("virtual", _append(), [1, 13])
    asm.move_result(3, True)
    asm.invoke("virtual", _m(BUILDER, "toString", STRING), [1])
    asm.move_result(0, True)
    asm.ret(0, "obj")
    asm.label("ingest_title")
    asm.const_string(0, "Need a title")
    asm.ret(0, "obj")
    asm.label("ingest_body")
    asm.const_string(0, "Need some text")
    asm.ret(0, "obj")


def _emit_lookup(asm, rt, index_desc):
    asm.invoke("static", _m(rt, "db", DB, (CONTEXT,)), [14])
    asm.move_result(0, True)
    asm.invoke("static", _m(rt, "ready", "V", (DB,)), [0])
    asm.invoke("static", _m(rt, "fill", "V", (DB,)), [0])
    asm.invoke("virtual", _m(DB, "close", "V"), [0])
    asm.invoke("static", _m(rt, "terms", STRING, (STRING,)), [15])
    asm.move_result(1, True)
    asm.const(2, 0)
    asm.const(3, 0)
    asm.invoke("virtual", _length(STRING), [1])
    asm.move_result(2, False)
    asm.if_zero("nez", 2, "lookup_sdk")
    asm.const(2, 0)
    asm.goto("lookup_fts")
    asm.label("lookup_sdk")
    asm.sget(2, ("Landroid/os/Build$VERSION;", "SDK_INT", "I"))
    asm.const(3, 31)
    asm.if_cmp("ge", 2, 3, "lookup_api")
    asm.const(2, 0)
    asm.goto("lookup_fts")
    asm.label("lookup_api")
    asm.const_string(2, "app_search")
    asm.invoke("virtual", _m(CONTEXT, "getSystemService", "Ljava/lang/Object;", (STRING,)), [14, 2])
    asm.move_result(2, True)
    asm.if_zero("nez", 2, "lookup_call")
    asm.const(2, 0)
    asm.goto("lookup_fts")
    asm.label("lookup_call")
    asm.invoke("static", _m(index_desc, "query", STRING, (CONTEXT, STRING)), [14, 15])
    asm.move_result(2, True)
    asm.if_zero("nez", 2, "lookup_hit")
    asm.const(2, 0)
    asm.goto("lookup_fts")
    asm.label("lookup_hit")
    asm.ret(2, "obj")
    asm.label("lookup_fts")
    asm.invoke("static", _m(rt, "db", DB, (CONTEXT,)), [14])
    asm.move_result(0, True)
    asm.invoke("static", _m(rt, "rows", STRING, (DB, STRING)), [0, 1])
    asm.move_result(2, True)
    asm.invoke("virtual", _m(DB, "close", "V"), [0])
    asm.ret(2, "obj")


def _emit_prop(asm):
    asm.new_instance(0, STRING_PROP_BUILDER)
    asm.invoke("direct", _m(STRING_PROP_BUILDER, "<init>", "V", (STRING,)), [0, 6])
    asm.const(1, 3)
    asm.invoke(
        "virtual",
        _m(STRING_PROP_BUILDER, "setCardinality", STRING_PROP_BUILDER, ("I",)),
        [0, 1],
    )
    asm.move_result(0, True)
    asm.invoke(
        "virtual",
        _m(STRING_PROP_BUILDER, "setIndexingType", STRING_PROP_BUILDER, ("I",)),
        [0, 7],
    )
    asm.move_result(0, True)
    asm.const(1, 1)
    asm.invoke(
        "virtual",
        _m(STRING_PROP_BUILDER, "setTokenizerType", STRING_PROP_BUILDER, ("I",)),
        [0, 1],
    )
    asm.move_result(0, True)
    asm.invoke("virtual", _m(STRING_PROP_BUILDER, "build", STRING_PROP), [0])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_schema(asm, index_desc):
    asm.new_instance(0, SCHEMA_BUILDER)
    asm.const_string(1, "Doc")
    asm.invoke("direct", _m(SCHEMA_BUILDER, "<init>", "V", (STRING,)), [0, 1])
    for name, indexing in (("title", 2), ("body", 2), ("kind", 1)):
        asm.const_string(1, name)
        asm.const(2, indexing)
        asm.invoke("static", _m(index_desc, "prop", STRING_PROP, (STRING, "I")), [1, 2])
        asm.move_result(3, True)
        asm.invoke("virtual", _m(SCHEMA_BUILDER, "addProperty", SCHEMA_BUILDER, (PROP,)), [0, 3])
        asm.move_result(0, True)
    asm.invoke("virtual", _m(SCHEMA_BUILDER, "build", SCHEMA), [0])
    asm.move_result(1, True)
    asm.new_instance(0, SET_SCHEMA_BUILDER)
    asm.invoke("direct", _m(SET_SCHEMA_BUILDER, "<init>", "V"), [0])
    asm.const(2, 1)
    asm.invoke(
        "virtual",
        _m(SET_SCHEMA_BUILDER, "setForceOverride", SET_SCHEMA_BUILDER, ("Z",)),
        [0, 2],
    )
    asm.move_result(0, True)
    asm.const(2, 1)
    asm.new_array(3, 2, "[" + SCHEMA)
    asm.const(2, 0)
    asm.aput_object(1, 3, 2)
    asm.invoke(
        "virtual",
        _m(SET_SCHEMA_BUILDER, "addSchemas", SET_SCHEMA_BUILDER, ("[" + SCHEMA,)),
        [0, 3],
    )
    asm.move_result(0, True)
    asm.invoke("virtual", _m(SET_SCHEMA_BUILDER, "build", SET_SCHEMA), [0])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_add_doc(asm):
    _blank(asm, 8, "doc_id")
    _blank(asm, 9, "doc_title")
    _blank(asm, 10, "doc_body")
    _blank(asm, 11, "doc_kind")
    asm.new_instance(0, BUILDER)
    asm.invoke("direct", _m(BUILDER, "<init>", "V"), [0])
    asm.const_string(1, "d")
    asm.invoke("virtual", _append(), [0, 1])
    asm.move_result(2, True)
    asm.invoke("virtual", _append(), [0, 8])
    asm.move_result(2, True)
    asm.invoke("virtual", _m(BUILDER, "toString", STRING), [0])
    asm.move_result(2, True)
    asm.new_instance(0, DOC_BUILDER)
    asm.const_string(1, "library")
    asm.const_string(3, "Doc")
    asm.invoke("direct", _m(DOC_BUILDER, "<init>", "V", (STRING, STRING, STRING)), [0, 1, 2, 3])
    for name, reg in (("title", 9), ("body", 10), ("kind", 11)):
        asm.const_string(1, name)
        asm.const(4, 1)
        asm.new_array(5, 4, "[Ljava/lang/String;")
        asm.const(4, 0)
        asm.aput_object(reg, 5, 4)
        asm.invoke(
            "virtual",
            _m(DOC_BUILDER, "setPropertyString", DOC_BUILDER, (STRING, "[Ljava/lang/String;")),
            [0, 1, 5],
        )
        asm.move_result(0, True)
    asm.invoke("virtual", _m(DOC_BUILDER, "build", DOC), [0])
    asm.move_result(0, True)
    asm.const(1, 1)
    asm.new_array(2, 1, "[" + DOC)
    asm.const(1, 0)
    asm.aput_object(0, 2, 1)
    asm.invoke(
        "virtual",
        _m(PUT_BUILDER, "addGenericDocuments", PUT_BUILDER, ("[" + DOC,)),
        [7, 2],
    )
    asm.move_result(0, True)
    asm.ret(0, "void")


def _emit_documents(asm, rt, index_desc):
    asm.invoke("static", _m(rt, "db", DB, (CONTEXT,)), [15])
    asm.move_result(7, True)
    asm.invoke("static", _m(rt, "ready", "V", (DB,)), [7])
    asm.new_instance(1, PUT_BUILDER)
    asm.invoke("direct", _m(PUT_BUILDER, "<init>", "V"), [1])
    asm.const_string(2, "SELECT CAST(rowid AS TEXT), title, body, kind FROM docs")
    asm.const(0, 0)
    asm.invoke("virtual", _m(DB, "rawQuery", CURSOR, (STRING, "[Ljava/lang/String;")), [7, 2, 0])
    asm.move_result(0, True)
    asm.const_string(2, "")
    asm.const_string(3, "")
    asm.const_string(4, "")
    asm.const_string(5, "")
    asm.const_string(6, "")
    asm.const(8, 0)
    asm.label("docs")
    asm.invoke("interface", _m(CURSOR, "moveToNext", "Z"), [0])
    asm.move_result(8, False)
    asm.if_zero("eqz", 8, "docs_done")
    asm.const(8, 0)
    asm.invoke("interface", _m(CURSOR, "getString", STRING, ("I",)), [0, 8])
    asm.move_result(3, True)
    asm.const(8, 1)
    asm.invoke("interface", _m(CURSOR, "getString", STRING, ("I",)), [0, 8])
    asm.move_result(4, True)
    asm.const(8, 2)
    asm.invoke("interface", _m(CURSOR, "getString", STRING, ("I",)), [0, 8])
    asm.move_result(5, True)
    asm.const(8, 3)
    asm.invoke("interface", _m(CURSOR, "getString", STRING, ("I",)), [0, 8])
    asm.move_result(6, True)
    asm.invoke(
        "static",
        _m(index_desc, "addDoc", "V", (PUT_BUILDER, STRING, STRING, STRING, STRING)),
        [1, 3, 4, 5, 6],
    )
    asm.goto("docs")
    asm.label("docs_done")
    asm.invoke("interface", _m(CURSOR, "close", "V"), [0])
    asm.invoke("virtual", _m(DB, "close", "V"), [7])
    asm.invoke("virtual", _m(PUT_BUILDER, "build", PUT), [1])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_spec(asm):
    asm.new_instance(0, SPEC_BUILDER)
    asm.invoke("direct", _m(SPEC_BUILDER, "<init>", "V"), [0])
    asm.const(1, 2)
    asm.invoke("virtual", _m(SPEC_BUILDER, "setTermMatch", SPEC_BUILDER, ("I",)), [0, 1])
    asm.move_result(0, True)
    asm.const_string(1, "Doc")
    asm.const(2, 1)
    asm.new_array(3, 2, "[Ljava/lang/String;")
    asm.const(2, 0)
    asm.aput_object(1, 3, 2)
    asm.invoke(
        "virtual",
        _m(SPEC_BUILDER, "addFilterSchemas", SPEC_BUILDER, ("[Ljava/lang/String;",)),
        [0, 3],
    )
    asm.move_result(0, True)
    asm.const(1, 10000)
    asm.invoke("virtual", _m(SPEC_BUILDER, "setSnippetCount", SPEC_BUILDER, ("I",)), [0, 1])
    asm.move_result(0, True)
    asm.invoke("virtual", _m(SPEC_BUILDER, "setSnippetCountPerProperty", SPEC_BUILDER, ("I",)), [0, 1])
    asm.move_result(0, True)
    asm.invoke("virtual", _m(SPEC_BUILDER, "setMaxSnippetSize", SPEC_BUILDER, ("I",)), [0, 1])
    asm.move_result(0, True)
    asm.invoke("virtual", _m(SPEC_BUILDER, "setResultCountPerPage", SPEC_BUILDER, ("I",)), [0, 1])
    asm.move_result(0, True)
    asm.invoke("virtual", _m(SPEC_BUILDER, "build", SPEC), [0])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_bare(asm):
    asm.new_instance(4, BUILDER)
    asm.invoke("direct", _m(BUILDER, "<init>", "V"), [4])
    asm.invoke("virtual", _append(), [4, 13])
    asm.move_result(2, True)
    asm.const_string(6, "  ·  ")
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(2, True)
    asm.invoke("virtual", _append(), [4, 12])
    asm.move_result(2, True)
    asm.invoke("virtual", _m(BUILDER, "toString", STRING), [4])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_line(asm, rt):
    # v15 is one search hit. Each match is its own row.
    asm.if_zero("nez", 15, "line_hit")
    asm.const_string(0, "")
    asm.ret(0, "obj")
    asm.label("line_hit")
    asm.invoke("virtual", _m(HIT, "getGenericDocument", DOC), [15])
    asm.move_result(14, True)
    asm.if_zero("nez", 14, "line_doc")
    asm.const_string(0, "")
    asm.ret(0, "obj")
    asm.label("line_doc")
    asm.const_string(0, "title")
    asm.invoke("virtual", _m(DOC, "getPropertyString", STRING, (STRING,)), [14, 0])
    asm.move_result(13, True)
    _blank(asm, 13, "line_title")
    asm.const_string(0, "kind")
    asm.invoke("virtual", _m(DOC, "getPropertyString", STRING, (STRING,)), [14, 0])
    asm.move_result(12, True)
    _blank(asm, 12, "line_kind")
    asm.invoke("virtual", _m(HIT, "getMatchInfos", "Ljava/util/List;"), [15])
    asm.move_result(11, True)
    asm.if_zero("eqz", 11, "line_none")
    asm.invoke("interface", _m("Ljava/util/List;", "size", "I"), [11])
    asm.move_result(0, False)
    asm.const(5, 0)
    asm.if_cmp("le", 0, 5, "line_none_hit")
    asm.new_instance(4, BUILDER)
    asm.invoke("direct", _m(BUILDER, "<init>", "V"), [4])
    asm.const(2, 0)
    asm.const(3, 0)
    asm.const(5, 0)
    asm.const(7, 0)
    asm.const(8, 0)
    asm.const(9, 0)
    asm.const(10, 0)
    asm.const_string(1, "")
    asm.const_string(6, "")
    asm.label("line_loop")
    asm.if_cmp("ge", 2, 0, "line_done")
    asm.invoke("interface", _m("Ljava/util/List;", "get", "Ljava/lang/Object;", ("I",)), [11, 2])
    asm.move_result(3, True)
    asm.if_zero("eqz", 3, "line_next")
    asm.check_cast(3, MATCH)
    asm.invoke("virtual", _m(MATCH, "getPropertyPath", STRING), [3])
    asm.move_result(6, True)
    asm.if_zero("eqz", 6, "line_next")
    asm.invoke("virtual", _m(DOC, "getPropertyString", STRING, (STRING,)), [14, 6])
    asm.move_result(7, True)
    asm.if_zero("eqz", 7, "line_next")
    asm.invoke("virtual", _m(MATCH, "getExactMatchRange", RANGE), [3])
    asm.move_result(8, True)
    asm.if_zero("eqz", 8, "line_next")
    asm.invoke("virtual", _m(RANGE, "getStart", "I"), [8])
    asm.move_result(9, False)
    asm.invoke("virtual", _m(RANGE, "getEnd", "I"), [8])
    asm.move_result(10, False)
    asm.invoke("static", _m(rt, "span", STRING, (STRING, "I", "I")), [7, 9, 10])
    asm.move_result(6, True)
    asm.if_zero("eqz", 6, "line_next")
    asm.invoke("virtual", _length(STRING), [6])
    asm.move_result(5, False)
    asm.if_zero("eqz", 5, "line_next")
    asm.invoke("virtual", _length(BUILDER), [4])
    asm.move_result(5, False)
    asm.if_zero("eqz", 5, "line_add")
    asm.const_string(1, "\n")
    asm.invoke("virtual", _append(), [4, 1])
    asm.move_result(1, True)
    asm.label("line_add")
    asm.invoke("virtual", _append(), [4, 13])
    asm.move_result(1, True)
    asm.const_string(1, "  ·  ")
    asm.invoke("virtual", _append(), [4, 1])
    asm.move_result(1, True)
    asm.invoke("virtual", _append(), [4, 12])
    asm.move_result(1, True)
    asm.const_string(1, "  —  ")
    asm.invoke("virtual", _append(), [4, 1])
    asm.move_result(1, True)
    asm.invoke("virtual", _append(), [4, 6])
    asm.move_result(1, True)
    asm.label("line_next")
    asm.const(5, 1)
    asm.binop(0x90, 2, 2, 5, "add-int")
    asm.goto("line_loop")
    asm.label("line_done")
    asm.invoke("virtual", _length(BUILDER), [4])
    asm.move_result(5, False)
    asm.if_zero("nez", 5, "line_out")
    _emit_bare(asm)
    asm.label("line_out")
    asm.invoke("virtual", _m(BUILDER, "toString", STRING), [4])
    asm.move_result(0, True)
    asm.ret(0, "obj")
    asm.label("line_none")
    _emit_bare(asm)
    asm.label("line_none_hit")
    _emit_bare(asm)


def _emit_lines(asm, index_desc):
    asm.if_zero("nez", 15, "page_have")
    asm.const_string(0, "")
    asm.ret(0, "obj")
    asm.label("page_have")
    asm.invoke("interface", _m("Ljava/util/List;", "size", "I"), [15])
    asm.move_result(0, False)
    asm.new_instance(1, BUILDER)
    asm.invoke("direct", _m(BUILDER, "<init>", "V"), [1])
    asm.const(2, 0)
    asm.const_string(7, "\n")
    asm.const(3, 0)
    asm.const(4, 0)
    asm.const(5, 0)
    asm.const(6, 0)
    asm.label("page")
    asm.if_cmp("ge", 2, 0, "page_done")
    asm.invoke("interface", _m("Ljava/util/List;", "get", "Ljava/lang/Object;", ("I",)), [15, 2])
    asm.move_result(3, True)
    asm.if_zero("eqz", 3, "page_next")
    asm.check_cast(3, HIT)
    asm.invoke("static", _m(index_desc, "line", STRING, (HIT,)), [3])
    asm.move_result(4, True)
    asm.if_zero("eqz", 4, "page_next")
    asm.invoke("virtual", _length(BUILDER), [1])
    asm.move_result(5, False)
    asm.if_zero("eqz", 5, "page_add")
    asm.invoke("virtual", _append(), [1, 7])
    asm.move_result(6, True)
    asm.label("page_add")
    asm.invoke("virtual", _append(), [1, 4])
    asm.move_result(6, True)
    asm.label("page_next")
    asm.const(5, 1)
    asm.binop(0x90, 2, 2, 5, "add-int")
    asm.goto("page")
    asm.label("page_done")
    asm.invoke("virtual", _m(BUILDER, "toString", STRING), [1])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_query(asm, cb):
    asm.const_string(0, "app_search")
    asm.invoke("virtual", _m(CONTEXT, "getSystemService", "Ljava/lang/Object;", (STRING,)), [14, 0])
    asm.move_result(0, True)
    asm.if_zero("nez", 0, "query_go")
    asm.const(0, 0)
    asm.ret(0, "obj")
    asm.label("query_go")
    asm.check_cast(0, MANAGER)
    asm.invoke(
        "static",
        _m("Ljava/util/concurrent/Executors;", "newSingleThreadExecutor", "Ljava/util/concurrent/ExecutorService;"),
        [],
    )
    asm.move_result(1, True)
    asm.const(2, 1)
    asm.new_instance(3, "Ljava/util/concurrent/CountDownLatch;")
    asm.invoke("direct", _m("Ljava/util/concurrent/CountDownLatch;", "<init>", "V", ("I",)), [3, 2])
    asm.new_instance(4, cb)
    asm.invoke("direct", _m(cb, "<init>", "V"), [4])
    asm.iput_obj(3, 4, (cb, "latch", "Ljava/util/concurrent/CountDownLatch;"))
    asm.iput_obj(1, 4, (cb, "executor", "Ljava/util/concurrent/Executor;"))
    asm.iput_obj(15, 4, (cb, "query", STRING))
    asm.iput_obj(14, 4, (cb, "context", "Ljava/lang/Object;"))
    asm.const(2, 0)
    asm.iput(2, 4, (cb, "stage", "I"))
    asm.new_instance(5, SEARCH_CONTEXT_BUILDER)
    asm.const_string(6, "library")
    asm.invoke("direct", _m(SEARCH_CONTEXT_BUILDER, "<init>", "V", (STRING,)), [5, 6])
    asm.invoke("virtual", _m(SEARCH_CONTEXT_BUILDER, "build", SEARCH_CONTEXT), [5])
    asm.move_result(5, True)
    asm.invoke(
        "virtual",
        _m(MANAGER, "createSearchSession", "V", (
            SEARCH_CONTEXT, "Ljava/util/concurrent/Executor;", CONSUMER,
        )),
        [0, 5, 1, 4],
    )
    asm.const_wide(6, 8)
    asm.const_string(8, "SECONDS")
    asm.invoke(
        "static",
        _m("Ljava/util/concurrent/TimeUnit;", "valueOf", "Ljava/util/concurrent/TimeUnit;", (STRING,)),
        [8],
    )
    asm.move_result(8, True)
    asm.invoke(
        "virtual",
        _m("Ljava/util/concurrent/CountDownLatch;", "await", "Z", ("J", "Ljava/util/concurrent/TimeUnit;")),
        [3, 6, 7, 8],
    )
    asm.move_result(9, False)
    asm.check_cast(1, "Ljava/util/concurrent/ExecutorService;")
    asm.invoke("interface", _m("Ljava/util/concurrent/ExecutorService;", "shutdown", "V"), [1])
    asm.iget_obj(0, 4, (cb, "text", STRING))
    asm.ret(0, "obj")


def _emit_finish(asm, cb):
    asm.iget_obj(0, 5, (cb, "hits", "Ljava/lang/Object;"))
    asm.if_zero("eqz", 0, "finish_session")
    asm.check_cast(0, HITS)
    asm.invoke("virtual", _m(HITS, "close", "V"), [0])
    asm.label("finish_session")
    asm.iget_obj(0, 5, (cb, "session", "Ljava/lang/Object;"))
    asm.if_zero("eqz", 0, "finish_latch")
    asm.check_cast(0, SESSION)
    asm.invoke("virtual", _m(SESSION, "close", "V"), [0])
    asm.label("finish_latch")
    asm.iget_obj(0, 5, (cb, "latch", "Ljava/util/concurrent/CountDownLatch;"))
    asm.invoke("virtual", _m("Ljava/util/concurrent/CountDownLatch;", "countDown", "V"), [0])
    asm.ret(0, "void")


def _emit_stop(asm, cb):
    asm.invoke("virtual", _m(cb, "finish", "V"), [14])
    asm.ret(0, "void")


def _emit_accept(asm, index_desc, cb):
    asm.if_zero("nez", 15, "accept_value")
    _emit_stop(asm, cb)
    asm.label("accept_value")
    asm.check_cast(15, RESULT)
    asm.invoke("virtual", _m(RESULT, "isSuccess", "Z"), [15])
    asm.move_result(0, False)
    asm.if_zero("nez", 0, "accept_ok")
    _emit_stop(asm, cb)
    asm.label("accept_ok")
    asm.invoke("virtual", _m(RESULT, "getResultValue", "Ljava/lang/Object;"), [15])
    asm.move_result(1, True)
    asm.if_zero("nez", 1, "accept_payload")
    _emit_stop(asm, cb)
    asm.label("accept_payload")
    asm.iget(0, 14, (cb, "stage", "I"))
    asm.if_zero("nez", 0, "accept_later")
    asm.iput_obj(1, 14, (cb, "session", "Ljava/lang/Object;"))
    asm.const(0, 1)
    asm.iput(0, 14, (cb, "stage", "I"))
    asm.invoke("static", _m(index_desc, "schema", SET_SCHEMA), [])
    asm.move_result(2, True)
    asm.check_cast(1, SESSION)
    asm.iget_obj(3, 14, (cb, "executor", "Ljava/util/concurrent/Executor;"))
    asm.invoke(
        "virtual",
        _m(SESSION, "setSchema", "V", (
            SET_SCHEMA, "Ljava/util/concurrent/Executor;", "Ljava/util/concurrent/Executor;", CONSUMER,
        )),
        [1, 2, 3, 3, 14],
    )
    asm.ret(0, "void")
    asm.label("accept_later")
    asm.const(2, 1)
    asm.if_cmp("ne", 0, 2, "accept_page")
    asm.iget_obj(4, 14, (cb, "context", "Ljava/lang/Object;"))
    asm.check_cast(4, CONTEXT)
    asm.invoke("static", _m(index_desc, "documents", PUT, (CONTEXT,)), [4])
    asm.move_result(2, True)
    asm.iget_obj(1, 14, (cb, "session", "Ljava/lang/Object;"))
    asm.check_cast(1, SESSION)
    asm.iget_obj(3, 14, (cb, "executor", "Ljava/util/concurrent/Executor;"))
    asm.invoke(
        "virtual",
        _m(SESSION, "put", "V", (PUT, "Ljava/util/concurrent/Executor;", BATCH_CB)),
        [1, 2, 3, 14],
    )
    asm.ret(0, "void")
    asm.label("accept_page")
    asm.check_cast(1, "Ljava/util/List;")
    asm.const_string(4, "")
    asm.invoke("interface", _m("Ljava/util/List;", "size", "I"), [1])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "accept_stop")
    asm.invoke("static", _m(index_desc, "lines", STRING, ("Ljava/util/List;",)), [1])
    asm.move_result(2, True)
    asm.if_zero("eqz", 2, "accept_more")
    asm.invoke("virtual", _length(STRING), [2])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "accept_more")
    asm.iget_obj(3, 14, (cb, "text", STRING))
    asm.if_zero("eqz", 3, "accept_store")
    asm.invoke("virtual", _length(STRING), [3])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "accept_store")
    asm.const_string(4, "\n")
    asm.invoke("virtual", _m(STRING, "concat", STRING, (STRING,)), [3, 4])
    asm.move_result(3, True)
    asm.invoke("virtual", _m(STRING, "concat", STRING, (STRING,)), [3, 2])
    asm.move_result(2, True)
    asm.label("accept_store")
    asm.iput_obj(2, 14, (cb, "text", STRING))
    asm.label("accept_more")
    asm.iget_obj(1, 14, (cb, "hits", "Ljava/lang/Object;"))
    asm.check_cast(1, HITS)
    asm.iget_obj(3, 14, (cb, "executor", "Ljava/util/concurrent/Executor;"))
    asm.invoke(
        "virtual",
        _m(HITS, "getNextPage", "V", ("Ljava/util/concurrent/Executor;", CONSUMER)),
        [1, 3, 14],
    )
    asm.ret(0, "void")
    asm.label("accept_stop")
    _emit_stop(asm, cb)


def _emit_on_result(asm, index_desc, cb):
    asm.if_zero("nez", 11, "put_ok")
    asm.invoke("virtual", _m(cb, "finish", "V"), [10])
    asm.ret(0, "void")
    asm.label("put_ok")
    asm.check_cast(11, BATCH)
    asm.invoke("virtual", _m(BATCH, "isSuccess", "Z"), [11])
    asm.move_result(0, False)
    asm.if_zero("nez", 0, "put_search")
    asm.invoke("virtual", _m(cb, "finish", "V"), [10])
    asm.ret(0, "void")
    asm.label("put_search")
    asm.invoke("static", _m(index_desc, "spec", SPEC), [])
    asm.move_result(2, True)
    asm.iget_obj(0, 10, (cb, "query", STRING))
    asm.iget_obj(1, 10, (cb, "session", "Ljava/lang/Object;"))
    asm.check_cast(1, SESSION)
    asm.invoke("virtual", _m(SESSION, "search", HITS, (STRING, SPEC)), [1, 0, 2])
    asm.move_result(2, True)
    asm.iput_obj(2, 10, (cb, "hits", "Ljava/lang/Object;"))
    asm.const(0, 2)
    asm.iput(0, 10, (cb, "stage", "I"))
    asm.iget_obj(3, 10, (cb, "executor", "Ljava/util/concurrent/Executor;"))
    asm.invoke(
        "virtual",
        _m(HITS, "getNextPage", "V", ("Ljava/util/concurrent/Executor;", CONSUMER)),
        [2, 3, 10],
    )
    asm.ret(0, "void")


def _emit_on_error(asm, cb):
    asm.invoke("virtual", _m(cb, "finish", "V"), [2])
    asm.ret(0, "void")
