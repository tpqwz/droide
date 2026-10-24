"""Search reads AppSearch when the device has it, and a built-in note list otherwise.

AppSearch is API 31. DroideRt.search checks SDK_INT and getSystemService("app_search")
and only then calls DroideSearch. That class is the only one whose methods name
android.app.appsearch, so an older device never resolves those types. A null result,
a timeout, or an empty query uses the same five notes. An empty query does not call
AppSearch: the platform rejects it, and the full list is what the form wants.
"""

from .dex import ACC_CONSTRUCTOR, ACC_PUBLIC, ACC_STATIC, Asm, ClassModel, MethodModel

NOTES = (
    "Ink is a deep blue mixed on the swatch",
    "Clay is a warm red left on the shelf",
    "Leaf is the green of a fresh note",
    "Sand is a pale gold hour",
    "Paper holds the private note from the device",
)
IDS = ("ink", "clay", "leaf", "sand", "paper")

MANAGER = "Landroid/app/appsearch/AppSearchManager;"
CONTEXT = "Landroid/app/appsearch/AppSearchManager$SearchContext;"
CONTEXT_BUILDER = "Landroid/app/appsearch/AppSearchManager$SearchContext$Builder;"
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
BATCH = "Landroid/app/appsearch/AppSearchBatchResult;"
BATCH_CB = "Landroid/app/appsearch/BatchResultCallback;"
CONSUMER = "Ljava/util/function/Consumer;"


def _class_desc(package, name):
    return "L%s/%s;" % (package.replace(".", "/"), name)


def _m(owner, name, ret, params=()):
    return (owner, name, ret, tuple(params))


def _append():
    return _m("Ljava/lang/StringBuilder;", "append", "Ljava/lang/StringBuilder;", ("Ljava/lang/String;",))


def _length():
    return _m("Ljava/lang/String;", "length", "I")


def _add(cls, name, ret, params, access, asm, registers, ins, outs):
    cls.method(MethodModel(name, ret, tuple(params), access, asm, registers, ins, outs))


def attach(dex, package, rt):
    rt_desc = _class_desc(package, "DroideRt")
    search_desc = _class_desc(package, "DroideSearch")
    cb_desc = _class_desc(package, "DroideSearch$Cb")

    keep = Asm()
    _emit_keep(keep)
    _add(rt, "keep", "V", (
        "Ljava/lang/StringBuilder;", "Ljava/lang/String;", "Ljava/lang/String;",
    ), ACC_PUBLIC | ACC_STATIC, keep, 8, 3, 2)

    notes = Asm()
    _emit_local_notes(notes, rt_desc)
    _add(rt, "localNotes", "Ljava/lang/String;", ("Ljava/lang/String;",),
         ACC_PUBLIC | ACC_STATIC, notes, 12, 1, 3)

    search = Asm()
    _emit_rt_search(search, rt_desc, search_desc)
    _add(rt, "search", "Ljava/lang/String;", (
        "Landroid/content/Context;", "Ljava/lang/String;",
    ), ACC_PUBLIC | ACC_STATIC, search, 16, 2, 2)

    api = ClassModel(search_desc, "Ljava/lang/Object;", ACC_PUBLIC, source="DroideSearch")
    schema = Asm()
    _emit_schema(schema)
    _add(api, "schema", SET_SCHEMA, (), ACC_PUBLIC | ACC_STATIC, schema, 8, 0, 2)
    one = Asm()
    _emit_note(one)
    _add(api, "note", "V", (PUT_BUILDER, "Ljava/lang/String;", "Ljava/lang/String;"),
         ACC_PUBLIC | ACC_STATIC, one, 8, 3, 4)
    docs = Asm()
    _emit_documents(docs, search_desc)
    _add(api, "documents", PUT, (), ACC_PUBLIC | ACC_STATIC, docs, 8, 0, 3)
    spec = Asm()
    _emit_spec(spec)
    _add(api, "spec", SPEC, (), ACC_PUBLIC | ACC_STATIC, spec, 8, 0, 2)
    lines = Asm()
    _emit_lines(lines)
    _add(api, "lines", "Ljava/lang/String;", ("Ljava/util/List;",),
         ACC_PUBLIC | ACC_STATIC, lines, 16, 1, 2)
    query = Asm()
    _emit_query(query, cb_desc)
    _add(api, "query", "Ljava/lang/String;", (
        "Landroid/content/Context;", "Ljava/lang/String;",
    ), ACC_PUBLIC | ACC_STATIC, query, 16, 2, 4)
    dex.add(api)

    cb = ClassModel(
        cb_desc,
        "Ljava/lang/Object;",
        ACC_PUBLIC,
        interfaces=[CONSUMER, BATCH_CB],
        source="DroideSearch",
    )
    cb.field("latch", "Ljava/util/concurrent/CountDownLatch;")
    cb.field("executor", "Ljava/util/concurrent/Executor;")
    cb.field("query", "Ljava/lang/String;")
    cb.field("stage", "I")
    cb.field("session", "Ljava/lang/Object;")
    cb.field("text", "Ljava/lang/String;")

    init = Asm()
    init.invoke("direct", _m("Ljava/lang/Object;", "<init>", "V"), [0])
    init.ret(0, "void")
    _add(cb, "<init>", "V", (), ACC_PUBLIC | ACC_CONSTRUCTOR, init, 1, 1, 1)

    finish = Asm()
    _emit_finish(finish, cb_desc)
    _add(cb, "finish", "V", (), ACC_PUBLIC, finish, 6, 1, 1)

    accept = Asm()
    _emit_accept(accept, search_desc, cb_desc)
    _add(cb, "accept", "V", ("Ljava/lang/Object;",), ACC_PUBLIC, accept, 16, 2, 5)

    on_result = Asm()
    _emit_on_result(on_result, search_desc, cb_desc)
    _add(cb, "onResult", "V", (BATCH,), ACC_PUBLIC, on_result, 12, 2, 3)

    on_error = Asm()
    on_error.invoke("virtual", _m(cb_desc, "finish", "V"), [2])
    on_error.ret(0, "void")
    _add(cb, "onSystemError", "V", ("Ljava/lang/Throwable;",), ACC_PUBLIC, on_error, 4, 2, 1)
    dex.add(cb)


def _emit_keep_line(asm, tag):
    """Append v6 to builder v5. Each path returns, so a reference does not join an unset register."""
    plain = "plain_" + tag
    asm.invoke("virtual", _m("Ljava/lang/StringBuilder;", "length", "I"), [5])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, plain)
    asm.const_string(1, "\n")
    asm.invoke("virtual", _append(), [5, 1])
    asm.move_result(2, True)
    asm.invoke("virtual", _append(), [5, 6])
    asm.move_result(2, True)
    asm.ret(0, "void")
    asm.label(plain)
    asm.invoke("virtual", _append(), [5, 6])
    asm.move_result(2, True)
    asm.ret(0, "void")


def _emit_keep(asm):
    """v5 builder, v6 line, v7 needle. An empty needle keeps every line."""
    asm.invoke("virtual", _length(), [7])
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "keep_all")
    asm.new_instance(1, "Ljava/util/Locale;")
    asm.const_string(2, "en")
    asm.invoke("direct", _m("Ljava/util/Locale;", "<init>", "V", ("Ljava/lang/String;",)), [1, 2])
    asm.invoke(
        "virtual",
        _m("Ljava/lang/String;", "toLowerCase", "Ljava/lang/String;", ("Ljava/util/Locale;",)),
        [6, 1],
    )
    asm.move_result(2, True)
    asm.invoke(
        "virtual",
        _m("Ljava/lang/String;", "contains", "Z", ("Ljava/lang/CharSequence;",)),
        [2, 7],
    )
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "keep_skip")
    _emit_keep_line(asm, "hit")
    asm.label("keep_all")
    _emit_keep_line(asm, "all")
    asm.label("keep_skip")
    asm.ret(0, "void")


def _emit_local_notes(asm, rt):
    """v11 is the query. A null query lists every note."""
    asm.if_zero("nez", 11, "notes_have")
    asm.const_string(11, "")
    asm.label("notes_have")
    asm.new_instance(1, "Ljava/util/Locale;")
    asm.const_string(2, "en")
    asm.invoke("direct", _m("Ljava/util/Locale;", "<init>", "V", ("Ljava/lang/String;",)), [1, 2])
    asm.invoke(
        "virtual",
        _m("Ljava/lang/String;", "toLowerCase", "Ljava/lang/String;", ("Ljava/util/Locale;",)),
        [11, 1],
    )
    asm.move_result(1, True)
    asm.new_instance(0, "Ljava/lang/StringBuilder;")
    asm.invoke("direct", _m("Ljava/lang/StringBuilder;", "<init>", "V"), [0])
    for note in NOTES:
        asm.const_string(2, note)
        asm.invoke(
            "static",
            _m(rt, "keep", "V", (
                "Ljava/lang/StringBuilder;", "Ljava/lang/String;", "Ljava/lang/String;",
            )),
            [0, 2, 1],
        )
    asm.invoke("virtual", _m("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;"), [0])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_rt_search(asm, rt, search_desc):
    """v14 context, v15 query. AppSearch types are not named in this method."""
    asm.if_zero("nez", 15, "search_len")
    asm.const(0, 0)
    asm.goto("search_local")
    asm.label("search_len")
    asm.invoke("virtual", _length(), [15])
    asm.move_result(0, False)
    asm.if_zero("nez", 0, "search_sdk")
    asm.const(0, 0)
    asm.goto("search_local")
    asm.label("search_sdk")
    asm.sget(0, ("Landroid/os/Build$VERSION;", "SDK_INT", "I"))
    asm.const(1, 31)
    asm.if_cmp("ge", 0, 1, "search_api")
    asm.const(0, 0)
    asm.goto("search_local")
    asm.label("search_api")
    asm.const_string(0, "app_search")
    asm.invoke(
        "virtual",
        _m("Landroid/content/Context;", "getSystemService", "Ljava/lang/Object;", ("Ljava/lang/String;",)),
        [14, 0],
    )
    asm.move_result(0, True)
    asm.if_zero("nez", 0, "search_service")
    asm.const(0, 0)
    asm.goto("search_local")
    asm.label("search_service")
    asm.invoke(
        "static",
        _m(search_desc, "query", "Ljava/lang/String;", (
            "Landroid/content/Context;", "Ljava/lang/String;",
        )),
        [14, 15],
    )
    asm.move_result(0, True)
    asm.if_zero("nez", 0, "search_hit")
    asm.const(0, 0)
    asm.goto("search_local")
    asm.label("search_hit")
    asm.ret(0, "obj")
    asm.label("search_local")
    asm.invoke("static", _m(rt, "localNotes", "Ljava/lang/String;", ("Ljava/lang/String;",)), [15])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_schema(asm):
    asm.new_instance(0, STRING_PROP_BUILDER)
    asm.const_string(1, "body")
    asm.invoke("direct", _m(STRING_PROP_BUILDER, "<init>", "V", ("Ljava/lang/String;",)), [0, 1])
    # CARDINALITY_REQUIRED = 3, INDEXING_TYPE_PREFIXES = 2, TOKENIZER_TYPE_PLAIN = 1.
    asm.const(1, 3)
    asm.invoke("virtual", _m(STRING_PROP_BUILDER, "setCardinality", STRING_PROP_BUILDER, ("I",)), [0, 1])
    asm.move_result(0, True)
    asm.const(1, 2)
    asm.invoke("virtual", _m(STRING_PROP_BUILDER, "setIndexingType", STRING_PROP_BUILDER, ("I",)), [0, 1])
    asm.move_result(0, True)
    asm.const(1, 1)
    asm.invoke("virtual", _m(STRING_PROP_BUILDER, "setTokenizerType", STRING_PROP_BUILDER, ("I",)), [0, 1])
    asm.move_result(0, True)
    asm.invoke("virtual", _m(STRING_PROP_BUILDER, "build", STRING_PROP), [0])
    asm.move_result(1, True)
    asm.new_instance(0, SCHEMA_BUILDER)
    asm.const_string(2, "Note")
    asm.invoke("direct", _m(SCHEMA_BUILDER, "<init>", "V", ("Ljava/lang/String;",)), [0, 2])
    asm.invoke("virtual", _m(SCHEMA_BUILDER, "addProperty", SCHEMA_BUILDER, (PROP,)), [0, 1])
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


def _emit_note(asm):
    """v5 request builder, v6 id, v7 body. Namespace notes, schema Note, property body."""
    asm.new_instance(0, DOC_BUILDER)
    asm.const_string(1, "notes")
    asm.const_string(2, "Note")
    asm.invoke(
        "direct",
        _m(DOC_BUILDER, "<init>", "V", ("Ljava/lang/String;", "Ljava/lang/String;", "Ljava/lang/String;")),
        [0, 1, 6, 2],
    )
    asm.const_string(1, "body")
    asm.const(2, 1)
    asm.new_array(3, 2, "[Ljava/lang/String;")
    asm.const(2, 0)
    asm.aput_object(7, 3, 2)
    asm.invoke(
        "virtual",
        _m(DOC_BUILDER, "setPropertyString", DOC_BUILDER, ("Ljava/lang/String;", "[Ljava/lang/String;")),
        [0, 1, 3],
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
        [5, 2],
    )
    asm.move_result(0, True)
    asm.ret(0, "void")


def _emit_documents(asm, search_desc):
    asm.new_instance(0, PUT_BUILDER)
    asm.invoke("direct", _m(PUT_BUILDER, "<init>", "V"), [0])
    for doc_id, note in zip(IDS, NOTES):
        asm.const_string(1, doc_id)
        asm.const_string(2, note)
        asm.invoke(
            "static",
            _m(search_desc, "note", "V", (PUT_BUILDER, "Ljava/lang/String;", "Ljava/lang/String;")),
            [0, 1, 2],
        )
    asm.invoke("virtual", _m(PUT_BUILDER, "build", PUT), [0])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_spec(asm):
    asm.new_instance(0, SPEC_BUILDER)
    asm.invoke("direct", _m(SPEC_BUILDER, "<init>", "V"), [0])
    # TERM_MATCH_PREFIX = 2. A query of "ink" matches the token "ink".
    asm.const(1, 2)
    asm.invoke("virtual", _m(SPEC_BUILDER, "setTermMatch", SPEC_BUILDER, ("I",)), [0, 1])
    asm.move_result(0, True)
    asm.const_string(1, "Note")
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
    asm.invoke("virtual", _m(SPEC_BUILDER, "build", SPEC), [0])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_lines(asm):
    """v15 is the page. One page holds the five notes. A null list is an empty string."""
    asm.if_zero("nez", 15, "rows_have")
    asm.const_string(0, "")
    asm.ret(0, "obj")
    asm.label("rows_have")
    asm.invoke("interface", _m("Ljava/util/List;", "size", "I"), [15])
    asm.move_result(0, False)
    asm.new_instance(1, "Ljava/lang/StringBuilder;")
    asm.invoke("direct", _m("Ljava/lang/StringBuilder;", "<init>", "V"), [1])
    asm.const(2, 0)
    asm.const_string(7, "\n")
    # const/4 0 joins the back edge, where these registers hold a reference or an int.
    asm.const(3, 0)
    asm.const(4, 0)
    asm.const(5, 0)
    asm.const(6, 0)
    asm.label("rows")
    asm.if_cmp("ge", 2, 0, "rows_done")
    asm.invoke("interface", _m("Ljava/util/List;", "get", "Ljava/lang/Object;", ("I",)), [15, 2])
    asm.move_result(3, True)
    asm.if_zero("eqz", 3, "rows_next")
    asm.check_cast(3, HIT)
    asm.invoke("virtual", _m(HIT, "getGenericDocument", DOC), [3])
    asm.move_result(3, True)
    asm.if_zero("eqz", 3, "rows_next")
    asm.const_string(4, "body")
    asm.invoke(
        "virtual",
        _m(DOC, "getPropertyString", "Ljava/lang/String;", ("Ljava/lang/String;",)),
        [3, 4],
    )
    asm.move_result(4, True)
    asm.if_zero("eqz", 4, "rows_next")
    asm.invoke("virtual", _m("Ljava/lang/StringBuilder;", "length", "I"), [1])
    asm.move_result(5, False)
    asm.if_zero("eqz", 5, "rows_add")
    asm.invoke("virtual", _append(), [1, 7])
    asm.move_result(6, True)
    asm.label("rows_add")
    asm.invoke("virtual", _append(), [1, 4])
    asm.move_result(6, True)
    asm.label("rows_next")
    asm.const(5, 1)
    asm.binop(0x90, 2, 2, 5, "add-int")
    asm.goto("rows")
    asm.label("rows_done")
    asm.invoke("virtual", _m("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;"), [1])
    asm.move_result(0, True)
    asm.ret(0, "obj")


def _emit_query(asm, cb):
    """v14 context, v15 query. The caller already checked the API. Null means the session failed."""
    asm.const_string(0, "app_search")
    asm.invoke(
        "virtual",
        _m("Landroid/content/Context;", "getSystemService", "Ljava/lang/Object;", ("Ljava/lang/String;",)),
        [14, 0],
    )
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
    asm.iput_obj(15, 4, (cb, "query", "Ljava/lang/String;"))
    asm.const(2, 0)
    asm.iput(2, 4, (cb, "stage", "I"))
    asm.new_instance(5, CONTEXT_BUILDER)
    asm.const_string(6, "notes")
    asm.invoke("direct", _m(CONTEXT_BUILDER, "<init>", "V", ("Ljava/lang/String;",)), [5, 6])
    asm.invoke("virtual", _m(CONTEXT_BUILDER, "build", CONTEXT), [5])
    asm.move_result(5, True)
    asm.invoke(
        "virtual",
        _m(MANAGER, "createSearchSession", "V", (CONTEXT, "Ljava/util/concurrent/Executor;", CONSUMER)),
        [0, 5, 1, 4],
    )
    # Eight seconds. v6 and v7 are the wide pair, and the callback thread is not this one.
    asm.const_wide(6, 8)
    asm.const_string(8, "SECONDS")
    asm.invoke(
        "static",
        _m("Ljava/util/concurrent/TimeUnit;", "valueOf", "Ljava/util/concurrent/TimeUnit;", ("Ljava/lang/String;",)),
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
    asm.iget_obj(0, 4, (cb, "text", "Ljava/lang/String;"))
    asm.ret(0, "obj")


def _emit_finish(asm, cb):
    """v5 is the callback. Close the session when one was opened, then release the latch."""
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


def _emit_accept(asm, search_desc, cb):
    """v14 callback, v15 AppSearchResult. Stage 0 opens the schema, 1 puts notes, 2 reads the page."""
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
    asm.invoke("static", _m(search_desc, "schema", SET_SCHEMA), [])
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
    asm.invoke("static", _m(search_desc, "documents", PUT), [])
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
    asm.invoke("static", _m(search_desc, "lines", "Ljava/lang/String;", ("Ljava/util/List;",)), [1])
    asm.move_result(1, True)
    asm.iput_obj(1, 14, (cb, "text", "Ljava/lang/String;"))
    _emit_stop(asm, cb)


def _emit_on_result(asm, search_desc, cb):
    """v10 callback, v11 batch result from put. A successful put searches and asks for one page."""
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
    asm.invoke("static", _m(search_desc, "spec", SPEC), [])
    asm.move_result(2, True)
    asm.iget_obj(0, 10, (cb, "query", "Ljava/lang/String;"))
    asm.iget_obj(1, 10, (cb, "session", "Ljava/lang/Object;"))
    asm.check_cast(1, SESSION)
    asm.invoke("virtual", _m(SESSION, "search", HITS, ("Ljava/lang/String;", SPEC)), [1, 0, 2])
    asm.move_result(2, True)
    asm.const(0, 2)
    asm.iput(0, 10, (cb, "stage", "I"))
    asm.iget_obj(3, 10, (cb, "executor", "Ljava/util/concurrent/Executor;"))
    asm.invoke(
        "virtual",
        _m(HITS, "getNextPage", "V", ("Ljava/util/concurrent/Executor;", CONSUMER)),
        [2, 3, 10],
    )
    asm.ret(0, "void")
