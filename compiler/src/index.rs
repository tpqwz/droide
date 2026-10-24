//! `Lookup` and `Ingest` keep a document library.
//!
//! Every document is stored in a SQLite FTS4 table. That works from API 24.
//! A non-empty lookup uses AppSearch when `SDK_INT` is at least 31 and the
//! `app_search` service exists. `DroideRt.lookup` does not name an AppSearch
//! type. `DroideIndex` is loaded only for that call, and `DroideIndex$Cb` is
//! created only inside `DroideIndex.query`. An empty query lists the library
//! through FTS4, because AppSearch rejects an empty string. A null result from
//! AppSearch means the session failed, so the same query runs against FTS4.
//! A non-null empty string is a successful search with no hits.
//!
//! Plain text, Markdown, HTML, CSV, and JSON are reduced to words before they
//! are indexed. Newlines are folded so one document stays one list row.
//!
//! A match reads `offsets()`, which lists every hit. FTS4 and FTS5 `snippet()`
//! keep a single window of at most 64 tokens, so `utfAt` and `span` walk the
//! offsets and keep each hit, on its own row, with the text around it.
//! AppSearch does the same from `MatchInfo` ranges and reads every page.

use crate::dex::{
    Asm, ClassModel, DexFile, FieldRef, MethodModel, MethodRef, ACC_CONSTRUCTOR, ACC_PUBLIC,
    ACC_STATIC,
};

const DB: &str = "Landroid/database/sqlite/SQLiteDatabase;";
const CURSOR: &str = "Landroid/database/Cursor;";
const FILE: &str = "Ljava/io/File;";
const CONTEXT: &str = "Landroid/content/Context;";
const STRING: &str = "Ljava/lang/String;";
const BUILDER: &str = "Ljava/lang/StringBuilder;";

const MANAGER: &str = "Landroid/app/appsearch/AppSearchManager;";
const SEARCH_CONTEXT: &str = "Landroid/app/appsearch/AppSearchManager$SearchContext;";
const SEARCH_CONTEXT_BUILDER: &str = "Landroid/app/appsearch/AppSearchManager$SearchContext$Builder;";
const SESSION: &str = "Landroid/app/appsearch/AppSearchSession;";
const RESULT: &str = "Landroid/app/appsearch/AppSearchResult;";
const SCHEMA: &str = "Landroid/app/appsearch/AppSearchSchema;";
const SCHEMA_BUILDER: &str = "Landroid/app/appsearch/AppSearchSchema$Builder;";
const PROP: &str = "Landroid/app/appsearch/AppSearchSchema$PropertyConfig;";
const STRING_PROP: &str = "Landroid/app/appsearch/AppSearchSchema$StringPropertyConfig;";
const STRING_PROP_BUILDER: &str =
    "Landroid/app/appsearch/AppSearchSchema$StringPropertyConfig$Builder;";
const SET_SCHEMA: &str = "Landroid/app/appsearch/SetSchemaRequest;";
const SET_SCHEMA_BUILDER: &str = "Landroid/app/appsearch/SetSchemaRequest$Builder;";
const PUT: &str = "Landroid/app/appsearch/PutDocumentsRequest;";
const PUT_BUILDER: &str = "Landroid/app/appsearch/PutDocumentsRequest$Builder;";
const DOC: &str = "Landroid/app/appsearch/GenericDocument;";
const DOC_BUILDER: &str = "Landroid/app/appsearch/GenericDocument$Builder;";
const SPEC: &str = "Landroid/app/appsearch/SearchSpec;";
const SPEC_BUILDER: &str = "Landroid/app/appsearch/SearchSpec$Builder;";
const HITS: &str = "Landroid/app/appsearch/SearchResults;";
const HIT: &str = "Landroid/app/appsearch/SearchResult;";
const MATCH: &str = "Landroid/app/appsearch/SearchResult$MatchInfo;";
const RANGE: &str = "Landroid/app/appsearch/SearchResult$MatchRange;";
const BATCH: &str = "Landroid/app/appsearch/AppSearchBatchResult;";
const BATCH_CB: &str = "Landroid/app/appsearch/BatchResultCallback;";
const CONSUMER: &str = "Ljava/util/function/Consumer;";

/// Characters of the column kept on each side of a hit. Both sides together
/// are wider than the 64-token `snippet()` window, and every hit is kept.
const SNIP_PAD: i64 = 256;

const SEEDS: &[(&str, &str, &str)] = &[
    (
        "Ink",
        "txt",
        "Ink is a deep blue mixed on the swatch and kept in a glass jar.",
    ),
    (
        "Clay",
        "md",
        "# Clay\n\nClay is a warm red left on the shelf.\n\n- dry\n- fired",
    ),
    (
        "Leaf",
        "html",
        "<h1>Leaf</h1><p>Leaf is the <b>green</b> of a fresh note.</p>",
    ),
    ("Hours", "csv", "name,hour\nSand,pale gold\nPaper,evening"),
    (
        "Device",
        "json",
        "{\"note\":\"Paper holds the private note from the device\",\"where\":\"shelf\"}",
    ),
];

pub fn attach(dex: &mut DexFile, package: &str, rt: &mut ClassModel) {
    let rt_desc = class_desc(package, "DroideRt");
    let index_desc = class_desc(package, "DroideIndex");
    let cb_desc = class_desc(package, "DroideIndex$Cb");

    let mut database = Asm::new();
    emit_db(&mut database);
    rt.method(method(
        "db",
        DB,
        vec![CONTEXT.into()],
        ACC_PUBLIC | ACC_STATIC,
        database,
        8,
        1,
        2,
    ));

    let mut schema = Asm::new();
    emit_ready(&mut schema);
    rt.method(method(
        "ready",
        "V",
        vec![DB.into()],
        ACC_PUBLIC | ACC_STATIC,
        schema,
        8,
        1,
        2,
    ));

    let mut text = Asm::new();
    emit_plain(&mut text);
    rt.method(method(
        "plain",
        STRING,
        vec![STRING.into(), STRING.into()],
        ACC_PUBLIC | ACC_STATIC,
        text,
        12,
        2,
        3,
    ));

    let mut row = Asm::new();
    emit_add_row(&mut row, &rt_desc);
    rt.method(method(
        "addRow",
        "V",
        vec![DB.into(), STRING.into(), STRING.into(), STRING.into()],
        ACC_PUBLIC | ACC_STATIC,
        row,
        12,
        4,
        3,
    ));

    let mut shelf = Asm::new();
    emit_fill(&mut shelf, &rt_desc);
    rt.method(method(
        "fill",
        "V",
        vec![DB.into()],
        ACC_PUBLIC | ACC_STATIC,
        shelf,
        8,
        1,
        4,
    ));

    let mut terms = Asm::new();
    emit_terms(&mut terms);
    rt.method(method(
        "terms",
        STRING,
        vec![STRING.into()],
        ACC_PUBLIC | ACC_STATIC,
        terms,
        12,
        1,
        3,
    ));

    let mut place = Asm::new();
    emit_utf_at(&mut place);
    rt.method(method(
        "utfAt",
        "I",
        vec![STRING.into(), "I".into()],
        ACC_PUBLIC | ACC_STATIC,
        place,
        8,
        2,
        2,
    ));

    let mut window = Asm::new();
    emit_span(&mut window);
    rt.method(method(
        "span",
        STRING,
        vec![STRING.into(), "I".into(), "I".into()],
        ACC_PUBLIC | ACC_STATIC,
        window,
        12,
        3,
        3,
    ));

    let mut clip = Asm::new();
    emit_snip(&mut clip, &rt_desc);
    rt.method(method(
        "snip",
        STRING,
        vec![STRING.into(), STRING.into(), "I".into()],
        ACC_PUBLIC | ACC_STATIC,
        clip,
        16,
        3,
        3,
    ));

    let mut poured = Asm::new();
    emit_pour(&mut poured);
    rt.method(method(
        "pour",
        "V",
        vec![BUILDER.into(), STRING.into(), STRING.into(), STRING.into()],
        ACC_PUBLIC | ACC_STATIC,
        poured,
        16,
        4,
        3,
    ));

    let mut listed = Asm::new();
    emit_rows(&mut listed, &rt_desc);
    rt.method(method(
        "rows",
        STRING,
        vec![DB.into(), STRING.into()],
        ACC_PUBLIC | ACC_STATIC,
        listed,
        16,
        2,
        4,
    ));

    let mut ingest = Asm::new();
    emit_ingest(&mut ingest, &rt_desc);
    rt.method(method(
        "ingest",
        STRING,
        vec![CONTEXT.into(), STRING.into(), STRING.into(), STRING.into()],
        ACC_PUBLIC | ACC_STATIC,
        ingest,
        16,
        4,
        4,
    ));

    let mut lookup = Asm::new();
    emit_lookup(&mut lookup, &rt_desc, &index_desc);
    rt.method(method(
        "lookup",
        STRING,
        vec![CONTEXT.into(), STRING.into()],
        ACC_PUBLIC | ACC_STATIC,
        lookup,
        16,
        2,
        2,
    ));

    let mut api = ClassModel::new(&index_desc, "Ljava/lang/Object;", ACC_PUBLIC, "DroideIndex");
    let mut request = Asm::new();
    emit_schema(&mut request, &index_desc);
    api.method(method(
        "schema",
        SET_SCHEMA,
        Vec::new(),
        ACC_PUBLIC | ACC_STATIC,
        request,
        8,
        0,
        3,
    ));
    let mut property = Asm::new();
    emit_prop(&mut property);
    api.method(method(
        "prop",
        STRING_PROP,
        vec![STRING.into(), "I".into()],
        ACC_PUBLIC | ACC_STATIC,
        property,
        8,
        2,
        2,
    ));
    let mut one = Asm::new();
    emit_add_doc(&mut one);
    api.method(method(
        "addDoc",
        "V",
        vec![
            PUT_BUILDER.into(),
            STRING.into(),
            STRING.into(),
            STRING.into(),
            STRING.into(),
        ],
        ACC_PUBLIC | ACC_STATIC,
        one,
        12,
        5,
        4,
    ));
    let mut docs = Asm::new();
    emit_documents(&mut docs, &rt_desc, &index_desc);
    api.method(method(
        "documents",
        PUT,
        vec![CONTEXT.into()],
        ACC_PUBLIC | ACC_STATIC,
        docs,
        16,
        1,
        5,
    ));
    let mut spec = Asm::new();
    emit_spec(&mut spec);
    api.method(method(
        "spec",
        SPEC,
        Vec::new(),
        ACC_PUBLIC | ACC_STATIC,
        spec,
        8,
        0,
        2,
    ));
    let mut hit = Asm::new();
    emit_line(&mut hit, &rt_desc);
    api.method(method(
        "line",
        STRING,
        vec![HIT.into()],
        ACC_PUBLIC | ACC_STATIC,
        hit,
        16,
        1,
        3,
    ));
    let mut lines = Asm::new();
    emit_lines(&mut lines, &index_desc);
    api.method(method(
        "lines",
        STRING,
        vec!["Ljava/util/List;".into()],
        ACC_PUBLIC | ACC_STATIC,
        lines,
        16,
        1,
        2,
    ));
    let mut query = Asm::new();
    emit_query(&mut query, &cb_desc);
    api.method(method(
        "query",
        STRING,
        vec![CONTEXT.into(), STRING.into()],
        ACC_PUBLIC | ACC_STATIC,
        query,
        16,
        2,
        4,
    ));
    dex.add(api);

    let mut cb = ClassModel::new(&cb_desc, "Ljava/lang/Object;", ACC_PUBLIC, "DroideIndex");
    cb.interfaces().push(CONSUMER.into());
    cb.interfaces().push(BATCH_CB.into());
    cb.field("latch", "Ljava/util/concurrent/CountDownLatch;", ACC_PUBLIC);
    cb.field("executor", "Ljava/util/concurrent/Executor;", ACC_PUBLIC);
    cb.field("query", STRING, ACC_PUBLIC);
    cb.field("context", "Ljava/lang/Object;", ACC_PUBLIC);
    cb.field("stage", "I", ACC_PUBLIC);
    cb.field("session", "Ljava/lang/Object;", ACC_PUBLIC);
    cb.field("hits", "Ljava/lang/Object;", ACC_PUBLIC);
    cb.field("text", STRING, ACC_PUBLIC);

    let mut init = Asm::new();
    init.invoke("direct", m("Ljava/lang/Object;", "<init>", "V", &[]), &[0]);
    init.ret(0, "void");
    cb.method(method(
        "<init>",
        "V",
        Vec::new(),
        ACC_PUBLIC | ACC_CONSTRUCTOR,
        init,
        1,
        1,
        1,
    ));

    let mut finish = Asm::new();
    emit_finish(&mut finish, &cb_desc);
    cb.method(method("finish", "V", Vec::new(), ACC_PUBLIC, finish, 6, 1, 1));

    let mut accept = Asm::new();
    emit_accept(&mut accept, &index_desc, &cb_desc);
    cb.method(method(
        "accept",
        "V",
        vec!["Ljava/lang/Object;".into()],
        ACC_PUBLIC,
        accept,
        16,
        2,
        5,
    ));

    let mut on_result = Asm::new();
    emit_on_result(&mut on_result, &index_desc, &cb_desc);
    cb.method(method(
        "onResult",
        "V",
        vec![BATCH.into()],
        ACC_PUBLIC,
        on_result,
        12,
        2,
        3,
    ));

    let mut on_error = Asm::new();
    emit_on_error(&mut on_error, &cb_desc);
    cb.method(method(
        "onSystemError",
        "V",
        vec!["Ljava/lang/Throwable;".into()],
        ACC_PUBLIC,
        on_error,
        4,
        2,
        1,
    ));
    dex.add(cb);
}

fn method(
    name: &str,
    ret: &str,
    params: Vec<String>,
    access: u32,
    asm: Asm,
    registers: u16,
    ins: u16,
    outs: u16,
) -> MethodModel {
    MethodModel::new(name, ret, params, access, Some(asm), registers, ins, outs)
}

fn class_desc(package: &str, name: &str) -> String {
    format!("L{}/{name};", package.replace('.', "/"))
}

fn m(owner: &str, name: &str, ret: &str, params: &[&str]) -> MethodRef {
    MethodRef {
        owner: owner.to_string(),
        name: name.to_string(),
        ret: ret.to_string(),
        params: params.iter().map(|item| (*item).to_string()).collect(),
    }
}

fn f(owner: &str, name: &str, ty: &str) -> FieldRef {
    FieldRef {
        owner: owner.to_string(),
        name: name.to_string(),
        ty: ty.to_string(),
    }
}

fn append() -> MethodRef {
    m(BUILDER, "append", BUILDER, &[STRING])
}

fn length_of(owner: &str) -> MethodRef {
    m(owner, "length", "I", &[])
}

fn replace_all(asm: &mut Asm, body: u16, pattern: &str, replacement: &str) {
    asm.const_string(0, pattern);
    asm.const_string(1, replacement);
    asm.invoke(
        "virtual",
        m(STRING, "replaceAll", STRING, &[STRING, STRING]),
        &[body, 0, 1],
    );
    asm.move_result(body, true);
}

fn replace(asm: &mut Asm, body: u16, pattern: &str, replacement: &str) {
    asm.const_string(0, pattern);
    asm.const_string(1, replacement);
    asm.invoke(
        "virtual",
        m(
            STRING,
            "replace",
            STRING,
            &["Ljava/lang/CharSequence;", "Ljava/lang/CharSequence;"],
        ),
        &[body, 0, 1],
    );
    asm.move_result(body, true);
}

fn jump_kind(asm: &mut Asm, text: &str, label: &str) {
    asm.const_string(1, text);
    asm.invoke(
        "virtual",
        m(STRING, "equals", "Z", &["Ljava/lang/Object;"]),
        &[11, 1],
    );
    asm.move_result(2, false);
    asm.if_zero("nez", 2, label);
}

fn fold(asm: &mut Asm, body: u16) {
    replace_all(asm, body, "\\s+", " ");
    asm.invoke("virtual", m(STRING, "trim", STRING, &[]), &[body]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v7` is the context. The databases directory is created when it is missing.
fn emit_db(asm: &mut Asm) {
    asm.const_string(0, "library.db");
    asm.invoke(
        "virtual",
        m(CONTEXT, "getDatabasePath", FILE, &[STRING]),
        &[7, 0],
    );
    asm.move_result(0, true);
    asm.invoke("virtual", m(FILE, "getParentFile", FILE, &[]), &[0]);
    asm.move_result(1, true);
    asm.invoke("virtual", m(FILE, "mkdirs", "Z", &[]), &[1]);
    asm.move_result(2, false);
    asm.invoke("virtual", m(FILE, "getPath", STRING, &[]), &[0]);
    asm.move_result(0, true);
    asm.const_(1, 0);
    asm.invoke(
        "static",
        m(
            DB,
            "openOrCreateDatabase",
            DB,
            &[STRING, "Landroid/database/sqlite/SQLiteDatabase$CursorFactory;"],
        ),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v7` is the database.
fn emit_ready(asm: &mut Asm) {
    asm.const_string(
        0,
        "CREATE VIRTUAL TABLE IF NOT EXISTS docs USING fts4(title, body, kind)",
    );
    asm.invoke("virtual", m(DB, "execSQL", "V", &[STRING]), &[7, 0]);
    asm.ret(0, "void");
}

/// `v10` body, `v11` kind. The stored text has no newlines, so a hit is one row.
fn emit_plain(asm: &mut Asm) {
    asm.if_zero("nez", 10, "plain_body");
    asm.const_string(10, "");
    asm.label("plain_body");
    asm.if_zero("nez", 11, "plain_kind");
    asm.const_string(11, "txt");
    asm.label("plain_kind");
    asm.new_instance(0, "Ljava/util/Locale;");
    asm.const_string(1, "en");
    asm.invoke(
        "direct",
        m("Ljava/util/Locale;", "<init>", "V", &[STRING]),
        &[0, 1],
    );
    asm.invoke(
        "virtual",
        m(STRING, "toLowerCase", STRING, &["Ljava/util/Locale;"]),
        &[11, 0],
    );
    asm.move_result(11, true);
    jump_kind(asm, "html", "plain_html");
    jump_kind(asm, "htm", "plain_html");
    jump_kind(asm, "md", "plain_md");
    jump_kind(asm, "markdown", "plain_md");
    jump_kind(asm, "csv", "plain_csv");
    jump_kind(asm, "json", "plain_json");
    fold(asm, 10);

    asm.label("plain_html");
    replace_all(asm, 10, "(?s)<[^>]*>", " ");
    replace(asm, 10, "&amp;", "&");
    replace(asm, 10, "&lt;", "<");
    replace(asm, 10, "&gt;", ">");
    replace(asm, 10, "&nbsp;", " ");
    fold(asm, 10);

    asm.label("plain_md");
    replace_all(asm, 10, "\\[([^\\]]+)\\]\\([^)]*\\)", "$1");
    replace_all(asm, 10, "[#*_`>]+", " ");
    fold(asm, 10);

    asm.label("plain_csv");
    replace(asm, 10, ",", " ");
    fold(asm, 10);

    asm.label("plain_json");
    replace_all(asm, 10, "[\\{\\}\\[\\]\":,]", " ");
    fold(asm, 10);
}

/// `v8` database, `v9` title, `v10` body, `v11` kind.
fn emit_add_row(asm: &mut Asm, rt: &str) {
    asm.invoke("static", m(rt, "plain", STRING, &[STRING, STRING]), &[10, 11]);
    asm.move_result(0, true);
    asm.const_string(1, "INSERT INTO docs(title, body, kind) VALUES(?,?,?)");
    asm.const_(2, 3);
    asm.new_array(3, 2, "[Ljava/lang/Object;");
    asm.const_(2, 0);
    asm.aput_object(9, 3, 2);
    asm.const_(2, 1);
    asm.aput_object(0, 3, 2);
    asm.const_(2, 2);
    asm.aput_object(11, 3, 2);
    asm.invoke(
        "virtual",
        m(DB, "execSQL", "V", &[STRING, "[Ljava/lang/Object;"]),
        &[8, 1, 3],
    );
    asm.ret(0, "void");
}

/// `v7` is the database. An empty library receives the five sample documents.
fn emit_fill(asm: &mut Asm, rt: &str) {
    asm.const_string(0, "SELECT count(*) FROM docs");
    asm.const_(1, 0);
    asm.invoke(
        "virtual",
        m(DB, "rawQuery", CURSOR, &[STRING, "[Ljava/lang/String;"]),
        &[7, 0, 1],
    );
    asm.move_result(0, true);
    asm.invoke("interface", m(CURSOR, "moveToNext", "Z", &[]), &[0]);
    asm.move_result(1, false);
    asm.const_(2, 0);
    asm.invoke("interface", m(CURSOR, "getInt", "I", &["I"]), &[0, 2]);
    asm.move_result(1, false);
    asm.invoke("interface", m(CURSOR, "close", "V", &[]), &[0]);
    asm.if_zero("nez", 1, "fill_done");
    for (title, kind, body) in SEEDS {
        asm.const_string(0, title);
        asm.const_string(1, body);
        asm.const_string(2, kind);
        asm.invoke(
            "static",
            m(rt, "addRow", "V", &[DB, STRING, STRING, STRING]),
            &[7, 0, 1, 2],
        );
    }
    asm.ret(0, "void");
    asm.label("fill_done");
    asm.ret(0, "void");
}

/// `v11` is the query. The result is an FTS prefix query, or "" to list every row.
fn emit_terms(asm: &mut Asm) {
    asm.if_zero("nez", 11, "terms_have");
    asm.const_string(0, "");
    asm.ret(0, "obj");
    asm.label("terms_have");
    asm.new_instance(0, "Ljava/util/Locale;");
    asm.const_string(1, "en");
    asm.invoke(
        "direct",
        m("Ljava/util/Locale;", "<init>", "V", &[STRING]),
        &[0, 1],
    );
    asm.invoke(
        "virtual",
        m(STRING, "toLowerCase", STRING, &["Ljava/util/Locale;"]),
        &[11, 0],
    );
    asm.move_result(11, true);
    replace_all(asm, 11, "[^a-z0-9]+", " ");
    asm.invoke("virtual", m(STRING, "trim", STRING, &[]), &[11]);
    asm.move_result(11, true);
    asm.invoke("virtual", length_of(STRING), &[11]);
    asm.move_result(2, false);
    asm.if_zero("nez", 2, "terms_star");
    asm.const_string(0, "");
    asm.ret(0, "obj");
    asm.label("terms_star");
    replace(asm, 11, " ", "* ");
    asm.const_string(0, "*");
    asm.invoke("virtual", m(STRING, "concat", STRING, &[STRING]), &[11, 0]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v6` is the column text and `v7` is a UTF-8 byte offset from `offsets()`.
/// The result is the corresponding Java char index. SQLite counts bytes, and
/// `String.substring` counts chars.
fn emit_utf_at(asm: &mut Asm) {
    asm.if_zero("nez", 6, "utf_text");
    asm.const_(0, 0);
    asm.ret(0, "int");
    asm.label("utf_text");
    asm.invoke("virtual", length_of(STRING), &[6]);
    asm.move_result(2, false);
    asm.const_(0, 0);
    asm.const_(1, 0);
    asm.const_(3, 0);
    asm.const_(4, 0);
    asm.label("utf_loop");
    asm.if_cmp("ge", 0, 2, "utf_done");
    asm.if_cmp("ge", 1, 7, "utf_done");
    asm.invoke("virtual", m(STRING, "charAt", "C", &["I"]), &[6, 0]);
    asm.move_result(3, false);
    asm.const_(4, 128);
    asm.if_cmp("lt", 3, 4, "utf_1");
    asm.const_(4, 2048);
    asm.if_cmp("lt", 3, 4, "utf_2");
    asm.const_(4, 0xD800);
    asm.if_cmp("lt", 3, 4, "utf_3");
    asm.const_(4, 0xDC00);
    asm.if_cmp("ge", 3, 4, "utf_3");
    asm.const_(4, 4);
    asm.binop(0x90, 1, 1, 4, "add-int");
    asm.const_(4, 1);
    asm.binop(0x90, 0, 0, 4, "add-int");
    asm.if_cmp("ge", 0, 2, "utf_done");
    asm.goto("utf_advance");
    asm.label("utf_1");
    asm.const_(4, 1);
    asm.goto("utf_add");
    asm.label("utf_2");
    asm.const_(4, 2);
    asm.goto("utf_add");
    asm.label("utf_3");
    asm.const_(4, 3);
    asm.label("utf_add");
    asm.binop(0x90, 1, 1, 4, "add-int");
    asm.label("utf_advance");
    asm.const_(4, 1);
    asm.binop(0x90, 0, 0, 4, "add-int");
    asm.goto("utf_loop");
    asm.label("utf_done");
    asm.ret(0, "int");
}

fn push_reg(asm: &mut Asm, reg: u16) {
    asm.invoke("virtual", append(), &[4, reg]);
    asm.move_result(11, true);
}

/// `v13` column text, `v14` the `offsets()` string, `v15` the column number.
/// Every hit in that column is kept. The match itself is wrapped in « ».
fn emit_snip(asm: &mut Asm, rt: &str) {
    asm.if_zero("nez", 13, "snip_text");
    asm.const_string(13, "");
    asm.label("snip_text");
    asm.if_zero("nez", 14, "snip_off");
    asm.const_string(14, "");
    asm.label("snip_off");
    asm.new_instance(4, BUILDER);
    asm.invoke("direct", m(BUILDER, "<init>", "V", &[]), &[4]);
    asm.const_(0, 0);
    asm.const_(1, 0);
    asm.const_(2, 0);
    asm.const_(3, 0);
    asm.const_(5, 0);
    asm.const_(7, 0);
    asm.const_(8, 0);
    asm.const_(9, 0);
    asm.const_(10, 0);
    asm.const_(12, 0);
    asm.const_string(6, "");
    asm.const_string(11, "");
    asm.label("snip_next");
    asm.invoke("virtual", length_of(STRING), &[14]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "snip_done");
    asm.const_string(6, " ");
    asm.invoke("virtual", m(STRING, "indexOf", "I", &[STRING]), &[14, 6]);
    asm.move_result(0, false);
    asm.const_(1, -1);
    asm.if_cmp("eq", 0, 1, "snip_last");
    asm.const_(2, 0);
    asm.invoke("virtual", m(STRING, "substring", STRING, &["I", "I"]), &[14, 2, 0]);
    asm.move_result(6, true);
    asm.const_(2, 1);
    asm.binop(0x90, 2, 0, 2, "add-int");
    asm.invoke("virtual", length_of(STRING), &[14]);
    asm.move_result(1, false);
    asm.invoke("virtual", m(STRING, "substring", STRING, &["I", "I"]), &[14, 2, 1]);
    asm.move_result(14, true);
    asm.goto("snip_parse");
    asm.label("snip_last");
    asm.move_(6, 14, true);
    asm.const_string(14, "");
    asm.label("snip_parse");
    asm.invoke(
        "static",
        m("Ljava/lang/Integer;", "parseInt", "I", &[STRING]),
        &[6],
    );
    asm.move_result(0, false);
    asm.const_(1, 0);
    asm.if_cmp("ne", 3, 1, "snip_ph1");
    asm.move_(7, 0, false);
    asm.goto("snip_step");
    asm.label("snip_ph1");
    asm.const_(1, 1);
    asm.if_cmp("ne", 3, 1, "snip_ph2");
    asm.goto("snip_step");
    asm.label("snip_ph2");
    asm.const_(1, 2);
    asm.if_cmp("ne", 3, 1, "snip_ph3");
    asm.move_(8, 0, false);
    asm.goto("snip_step");
    asm.label("snip_ph3");
    asm.move_(9, 0, false);
    asm.const_(1, 0);
    asm.if_cmp("ne", 7, 15, "snip_step");
    asm.if_cmp("lt", 8, 1, "snip_step");
    asm.if_cmp("le", 9, 1, "snip_step");
    asm.binop(0x90, 2, 8, 9, "add-int");
    asm.invoke("static", m(rt, "utfAt", "I", &[STRING, "I"]), &[13, 8]);
    asm.move_result(10, false);
    asm.invoke("static", m(rt, "utfAt", "I", &[STRING, "I"]), &[13, 2]);
    asm.move_result(12, false);
    asm.if_cmp("ge", 10, 12, "snip_step");
    asm.invoke("static", m(rt, "span", STRING, &[STRING, "I", "I"]), &[13, 10, 12]);
    asm.move_result(6, true);
    asm.if_zero("eqz", 6, "snip_step");
    asm.invoke("virtual", length_of(STRING), &[6]);
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "snip_step");
    asm.invoke("virtual", length_of(BUILDER), &[4]);
    asm.move_result(2, false);
    asm.if_zero("eqz", 2, "snip_add");
    asm.const_string(11, "\n");
    asm.invoke("virtual", append(), &[4, 11]);
    asm.move_result(11, true);
    asm.label("snip_add");
    push_reg(asm, 6);
    asm.goto("snip_step");
    asm.label("snip_step");
    asm.const_(1, 3);
    asm.if_cmp("ne", 3, 1, "snip_inc");
    asm.const_(3, 0);
    asm.goto("snip_next");
    asm.label("snip_inc");
    asm.const_(1, 1);
    asm.binop(0x90, 3, 3, 1, "add-int");
    asm.goto("snip_next");
    asm.label("snip_done");
    asm.invoke("virtual", m(BUILDER, "toString", STRING, &[]), &[4]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v9` text, `v10` start, `v11` end, in Java chars. One hit, marked.
fn emit_span(asm: &mut Asm) {
    asm.if_zero("nez", 9, "span_text");
    asm.const_string(9, "");
    asm.label("span_text");
    asm.invoke("virtual", length_of(STRING), &[9]);
    asm.move_result(5, false);
    asm.const_(1, 0);
    asm.if_cmp("ge", 10, 1, "span_start");
    asm.const_(10, 0);
    asm.label("span_start");
    asm.if_cmp("ge", 11, 10, "span_end0");
    asm.move_(11, 10, false);
    asm.label("span_end0");
    asm.if_cmp("le", 11, 5, "span_end1");
    asm.move_(11, 5, false);
    asm.label("span_end1");
    asm.if_cmp("lt", 10, 11, "span_hit");
    asm.const_string(0, "");
    asm.ret(0, "obj");
    asm.label("span_hit");
    asm.const_(0, SNIP_PAD);
    asm.binop(0x91, 0, 10, 0, "sub-int");
    asm.const_(1, 0);
    asm.if_cmp("ge", 0, 1, "span_ws");
    asm.const_(0, 0);
    asm.label("span_ws");
    asm.const_(1, SNIP_PAD);
    asm.binop(0x90, 1, 11, 1, "add-int");
    asm.if_cmp("le", 1, 5, "span_we");
    asm.move_(1, 5, false);
    asm.label("span_we");
    asm.new_instance(4, BUILDER);
    asm.invoke("direct", m(BUILDER, "<init>", "V", &[]), &[4]);
    asm.const_string(2, "");
    asm.const_string(6, "");
    asm.const_(3, 0);
    asm.if_cmp("le", 0, 3, "span_pre");
    asm.const_string(6, "…");
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(2, true);
    asm.label("span_pre");
    asm.if_cmp("ge", 0, 10, "span_mark");
    asm.invoke("virtual", m(STRING, "substring", STRING, &["I", "I"]), &[9, 0, 10]);
    asm.move_result(6, true);
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(2, true);
    asm.label("span_mark");
    asm.const_string(6, "«");
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(2, true);
    asm.invoke("virtual", m(STRING, "substring", STRING, &["I", "I"]), &[9, 10, 11]);
    asm.move_result(6, true);
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(2, true);
    asm.const_string(6, "»");
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(2, true);
    asm.if_cmp("ge", 11, 1, "span_tail");
    asm.invoke("virtual", m(STRING, "substring", STRING, &["I", "I"]), &[9, 11, 1]);
    asm.move_result(6, true);
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(2, true);
    asm.label("span_tail");
    asm.if_cmp("ge", 1, 5, "span_done");
    asm.const_string(6, "…");
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(2, true);
    asm.label("span_done");
    asm.invoke("virtual", m(BUILDER, "toString", STRING, &[]), &[4]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v12` result builder, `v13` title, `v14` kind, `v15` snippets, one per line.
fn emit_pour(asm: &mut Asm) {
    asm.if_zero("nez", 15, "pour_clips");
    asm.ret(0, "void");
    asm.label("pour_clips");
    asm.invoke("virtual", length_of(STRING), &[15]);
    asm.move_result(0, false);
    asm.if_zero("nez", 0, "pour_go");
    asm.ret(0, "void");
    asm.label("pour_go");
    asm.const_(0, 0);
    asm.const_(1, 0);
    asm.const_(3, 0);
    asm.const_(5, 0);
    asm.const_string(2, "");
    asm.const_string(6, "");
    asm.label("pour_next");
    asm.invoke("virtual", length_of(STRING), &[15]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "pour_done");
    asm.const_string(2, "\n");
    asm.invoke("virtual", m(STRING, "indexOf", "I", &[STRING]), &[15, 2]);
    asm.move_result(0, false);
    asm.const_(1, -1);
    asm.if_cmp("eq", 0, 1, "pour_last");
    asm.const_(1, 0);
    asm.invoke("virtual", m(STRING, "substring", STRING, &["I", "I"]), &[15, 1, 0]);
    asm.move_result(6, true);
    asm.const_(1, 1);
    asm.binop(0x90, 1, 0, 1, "add-int");
    asm.invoke("virtual", length_of(STRING), &[15]);
    asm.move_result(3, false);
    asm.invoke("virtual", m(STRING, "substring", STRING, &["I", "I"]), &[15, 1, 3]);
    asm.move_result(15, true);
    asm.goto("pour_line");
    asm.label("pour_last");
    asm.move_(6, 15, true);
    asm.const_string(15, "");
    asm.label("pour_line");
    asm.invoke("virtual", length_of(STRING), &[6]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "pour_next");
    asm.invoke("virtual", length_of(BUILDER), &[12]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "pour_add");
    asm.const_string(2, "\n");
    asm.invoke("virtual", append(), &[12, 2]);
    asm.move_result(2, true);
    asm.label("pour_add");
    asm.invoke("virtual", append(), &[12, 13]);
    asm.move_result(2, true);
    asm.const_string(2, "  ·  ");
    asm.invoke("virtual", append(), &[12, 2]);
    asm.move_result(2, true);
    asm.invoke("virtual", append(), &[12, 14]);
    asm.move_result(2, true);
    asm.const_string(2, "  —  ");
    asm.invoke("virtual", append(), &[12, 2]);
    asm.move_result(2, true);
    asm.invoke("virtual", append(), &[12, 6]);
    asm.move_result(2, true);
    asm.goto("pour_next");
    asm.label("pour_done");
    asm.ret(0, "void");
}

/// `v14` database, `v15` FTS query. An empty query lists titles. A match gives
/// one row for every hit in the title and the body.
fn emit_rows(asm: &mut Asm, rt: &str) {
    asm.new_instance(4, BUILDER);
    asm.invoke("direct", m(BUILDER, "<init>", "V", &[]), &[4]);
    asm.const_string(1, "\n");
    asm.const_string(2, "");
    asm.const_(3, 0);
    asm.const_(5, 0);
    asm.const_string(6, "");
    asm.invoke("virtual", length_of(STRING), &[15]);
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "rows_all");
    asm.const_string(
        2,
        "SELECT title, body, kind, offsets(docs) FROM docs WHERE docs MATCH ? ORDER BY rowid",
    );
    asm.const_(8, 1);
    asm.new_array(9, 8, "[Ljava/lang/String;");
    asm.const_(8, 0);
    asm.aput_object(15, 9, 8);
    asm.invoke(
        "virtual",
        m(DB, "rawQuery", CURSOR, &[STRING, "[Ljava/lang/String;"]),
        &[14, 2, 9],
    );
    asm.move_result(0, true);
    asm.const_(3, 0);
    asm.const_(5, 0);
    asm.const_string(2, "");
    asm.const_string(6, "");
    asm.const_string(7, "");
    asm.const_string(8, "");
    asm.const_string(10, "");
    asm.const_string(11, "");
    asm.const_string(12, "");
    asm.label("rows_hit");
    asm.invoke("interface", m(CURSOR, "moveToNext", "Z", &[]), &[0]);
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "rows_hit_done");
    asm.const_(3, 0);
    asm.invoke("interface", m(CURSOR, "getString", STRING, &["I"]), &[0, 3]);
    asm.move_result(6, true);
    blank(asm, 6, "rows_title");
    asm.const_(3, 1);
    asm.invoke("interface", m(CURSOR, "getString", STRING, &["I"]), &[0, 3]);
    asm.move_result(7, true);
    blank(asm, 7, "rows_body_text");
    asm.const_(3, 2);
    asm.invoke("interface", m(CURSOR, "getString", STRING, &["I"]), &[0, 3]);
    asm.move_result(8, true);
    blank(asm, 8, "rows_kind");
    asm.const_(3, 3);
    asm.invoke("interface", m(CURSOR, "getString", STRING, &["I"]), &[0, 3]);
    asm.move_result(12, true);
    blank(asm, 12, "rows_offsets");
    asm.const_(3, 0);
    asm.invoke("static", m(rt, "snip", STRING, &[STRING, STRING, "I"]), &[6, 12, 3]);
    asm.move_result(10, true);
    asm.const_(3, 1);
    asm.invoke("static", m(rt, "snip", STRING, &[STRING, STRING, "I"]), &[7, 12, 3]);
    asm.move_result(11, true);
    asm.invoke("virtual", length_of(STRING), &[10]);
    asm.move_result(3, false);
    asm.invoke("virtual", length_of(STRING), &[11]);
    asm.move_result(5, false);
    asm.binop(0x90, 3, 3, 5, "add-int");
    asm.if_zero("nez", 3, "rows_clips");
    asm.invoke("virtual", length_of(BUILDER), &[4]);
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "rows_plain");
    asm.invoke("virtual", append(), &[4, 1]);
    asm.move_result(2, true);
    asm.label("rows_plain");
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(2, true);
    asm.const_string(2, "  ·  ");
    asm.invoke("virtual", append(), &[4, 2]);
    asm.move_result(2, true);
    asm.invoke("virtual", append(), &[4, 8]);
    asm.move_result(2, true);
    asm.goto("rows_hit");
    asm.label("rows_clips");
    asm.invoke(
        "static",
        m(rt, "pour", "V", &[BUILDER, STRING, STRING, STRING]),
        &[4, 6, 8, 10],
    );
    asm.invoke(
        "static",
        m(rt, "pour", "V", &[BUILDER, STRING, STRING, STRING]),
        &[4, 6, 8, 11],
    );
    asm.goto("rows_hit");
    asm.label("rows_hit_done");
    asm.invoke("interface", m(CURSOR, "close", "V", &[]), &[0]);
    asm.invoke("virtual", m(BUILDER, "toString", STRING, &[]), &[4]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("rows_all");
    asm.const_string(2, "SELECT title || '  ·  ' || kind FROM docs ORDER BY rowid");
    asm.const_(9, 0);
    asm.invoke(
        "virtual",
        m(DB, "rawQuery", CURSOR, &[STRING, "[Ljava/lang/String;"]),
        &[14, 2, 9],
    );
    asm.move_result(0, true);
    asm.const_(3, 0);
    asm.const_(5, 0);
    asm.const_(8, 0);
    asm.const_(9, 0);
    asm.label("rows_scan");
    asm.invoke("interface", m(CURSOR, "moveToNext", "Z", &[]), &[0]);
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "rows_done");
    asm.const_(3, 0);
    asm.invoke("interface", m(CURSOR, "getString", STRING, &["I"]), &[0, 3]);
    asm.move_result(6, true);
    asm.if_zero("nez", 6, "rows_text");
    asm.const_string(6, "");
    asm.label("rows_text");
    asm.invoke("virtual", length_of(BUILDER), &[4]);
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "rows_add");
    asm.invoke("virtual", append(), &[4, 1]);
    asm.move_result(2, true);
    asm.label("rows_add");
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(2, true);
    asm.goto("rows_scan");
    asm.label("rows_done");
    asm.invoke("interface", m(CURSOR, "close", "V", &[]), &[0]);
    asm.invoke("virtual", m(BUILDER, "toString", STRING, &[]), &[4]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v12` context, `v13` title, `v14` body, `v15` kind.
fn emit_ingest(asm: &mut Asm, rt: &str) {
    asm.if_zero("eqz", 13, "ingest_title");
    asm.invoke("virtual", length_of(STRING), &[13]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "ingest_title");
    asm.if_zero("eqz", 14, "ingest_body");
    asm.invoke("virtual", length_of(STRING), &[14]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "ingest_body");
    asm.if_zero("eqz", 15, "ingest_kind");
    asm.invoke("virtual", length_of(STRING), &[15]);
    asm.move_result(0, false);
    asm.if_zero("nez", 0, "ingest_open");
    asm.label("ingest_kind");
    asm.const_string(15, "txt");
    asm.label("ingest_open");
    asm.invoke("static", m(rt, "db", DB, &[CONTEXT]), &[12]);
    asm.move_result(0, true);
    asm.invoke("static", m(rt, "ready", "V", &[DB]), &[0]);
    asm.invoke(
        "static",
        m(rt, "addRow", "V", &[DB, STRING, STRING, STRING]),
        &[0, 13, 14, 15],
    );
    asm.invoke("virtual", m(DB, "close", "V", &[]), &[0]);
    asm.new_instance(1, BUILDER);
    asm.invoke("direct", m(BUILDER, "<init>", "V", &[]), &[1]);
    asm.const_string(2, "Added ");
    asm.invoke("virtual", append(), &[1, 2]);
    asm.move_result(3, true);
    asm.invoke("virtual", append(), &[1, 13]);
    asm.move_result(3, true);
    asm.invoke("virtual", m(BUILDER, "toString", STRING, &[]), &[1]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("ingest_title");
    asm.const_string(0, "Need a title");
    asm.ret(0, "obj");
    asm.label("ingest_body");
    asm.const_string(0, "Need some text");
    asm.ret(0, "obj");
}

/// `v14` context, `v15` query. AppSearch types are not named in this method.
fn emit_lookup(asm: &mut Asm, rt: &str, index_desc: &str) {
    asm.invoke("static", m(rt, "db", DB, &[CONTEXT]), &[14]);
    asm.move_result(0, true);
    asm.invoke("static", m(rt, "ready", "V", &[DB]), &[0]);
    asm.invoke("static", m(rt, "fill", "V", &[DB]), &[0]);
    asm.invoke("virtual", m(DB, "close", "V", &[]), &[0]);
    asm.invoke("static", m(rt, "terms", STRING, &[STRING]), &[15]);
    asm.move_result(1, true);
    asm.const_(2, 0);
    asm.const_(3, 0);
    asm.invoke("virtual", length_of(STRING), &[1]);
    asm.move_result(2, false);
    asm.if_zero("nez", 2, "lookup_sdk");
    asm.const_(2, 0);
    asm.goto("lookup_fts");
    asm.label("lookup_sdk");
    asm.sget(2, f("Landroid/os/Build$VERSION;", "SDK_INT", "I"));
    asm.const_(3, 31);
    asm.if_cmp("ge", 2, 3, "lookup_api");
    asm.const_(2, 0);
    asm.goto("lookup_fts");
    asm.label("lookup_api");
    asm.const_string(2, "app_search");
    asm.invoke(
        "virtual",
        m(CONTEXT, "getSystemService", "Ljava/lang/Object;", &[STRING]),
        &[14, 2],
    );
    asm.move_result(2, true);
    asm.if_zero("nez", 2, "lookup_call");
    asm.const_(2, 0);
    asm.goto("lookup_fts");
    asm.label("lookup_call");
    asm.invoke(
        "static",
        m(index_desc, "query", STRING, &[CONTEXT, STRING]),
        &[14, 15],
    );
    asm.move_result(2, true);
    asm.if_zero("nez", 2, "lookup_hit");
    asm.const_(2, 0);
    asm.goto("lookup_fts");
    asm.label("lookup_hit");
    asm.ret(2, "obj");
    asm.label("lookup_fts");
    asm.invoke("static", m(rt, "db", DB, &[CONTEXT]), &[14]);
    asm.move_result(0, true);
    asm.invoke("static", m(rt, "rows", STRING, &[DB, STRING]), &[0, 1]);
    asm.move_result(2, true);
    asm.invoke("virtual", m(DB, "close", "V", &[]), &[0]);
    asm.ret(2, "obj");
}

/// `v6` name, `v7` indexing type. Cardinality is required and the tokenizer is plain.
fn emit_prop(asm: &mut Asm) {
    asm.new_instance(0, STRING_PROP_BUILDER);
    asm.invoke(
        "direct",
        m(STRING_PROP_BUILDER, "<init>", "V", &[STRING]),
        &[0, 6],
    );
    asm.const_(1, 3);
    asm.invoke(
        "virtual",
        m(STRING_PROP_BUILDER, "setCardinality", STRING_PROP_BUILDER, &["I"]),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.invoke(
        "virtual",
        m(STRING_PROP_BUILDER, "setIndexingType", STRING_PROP_BUILDER, &["I"]),
        &[0, 7],
    );
    asm.move_result(0, true);
    asm.const_(1, 1);
    asm.invoke(
        "virtual",
        m(STRING_PROP_BUILDER, "setTokenizerType", STRING_PROP_BUILDER, &["I"]),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.invoke("virtual", m(STRING_PROP_BUILDER, "build", STRING_PROP, &[]), &[0]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

fn emit_schema(asm: &mut Asm, index_desc: &str) {
    asm.new_instance(0, SCHEMA_BUILDER);
    asm.const_string(1, "Doc");
    asm.invoke(
        "direct",
        m(SCHEMA_BUILDER, "<init>", "V", &[STRING]),
        &[0, 1],
    );
    for (name, indexing) in [("title", 2i64), ("body", 2), ("kind", 1)] {
        asm.const_string(1, name);
        asm.const_(2, indexing);
        asm.invoke(
            "static",
            m(index_desc, "prop", STRING_PROP, &[STRING, "I"]),
            &[1, 2],
        );
        asm.move_result(3, true);
        asm.invoke(
            "virtual",
            m(SCHEMA_BUILDER, "addProperty", SCHEMA_BUILDER, &[PROP]),
            &[0, 3],
        );
        asm.move_result(0, true);
    }
    asm.invoke("virtual", m(SCHEMA_BUILDER, "build", SCHEMA, &[]), &[0]);
    asm.move_result(1, true);
    asm.new_instance(0, SET_SCHEMA_BUILDER);
    asm.invoke("direct", m(SET_SCHEMA_BUILDER, "<init>", "V", &[]), &[0]);
    asm.const_(2, 1);
    asm.invoke(
        "virtual",
        m(SET_SCHEMA_BUILDER, "setForceOverride", SET_SCHEMA_BUILDER, &["Z"]),
        &[0, 2],
    );
    asm.move_result(0, true);
    asm.const_(2, 1);
    asm.new_array(3, 2, &format!("[{SCHEMA}"));
    asm.const_(2, 0);
    asm.aput_object(1, 3, 2);
    asm.invoke(
        "virtual",
        m(SET_SCHEMA_BUILDER, "addSchemas", SET_SCHEMA_BUILDER, &[&format!("[{SCHEMA}")]),
        &[0, 3],
    );
    asm.move_result(0, true);
    asm.invoke("virtual", m(SET_SCHEMA_BUILDER, "build", SET_SCHEMA, &[]), &[0]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

fn blank(asm: &mut Asm, reg: u16, label: &str) {
    asm.if_zero("nez", reg, label);
    asm.const_string(reg, "");
    asm.label(label);
}

/// `v7` request builder, `v8` id, `v9` title, `v10` body, `v11` kind.
fn emit_add_doc(asm: &mut Asm) {
    blank(asm, 8, "doc_id");
    blank(asm, 9, "doc_title");
    blank(asm, 10, "doc_body");
    blank(asm, 11, "doc_kind");
    asm.new_instance(0, BUILDER);
    asm.invoke("direct", m(BUILDER, "<init>", "V", &[]), &[0]);
    asm.const_string(1, "d");
    asm.invoke("virtual", append(), &[0, 1]);
    asm.move_result(2, true);
    asm.invoke("virtual", append(), &[0, 8]);
    asm.move_result(2, true);
    asm.invoke("virtual", m(BUILDER, "toString", STRING, &[]), &[0]);
    asm.move_result(2, true);
    asm.new_instance(0, DOC_BUILDER);
    asm.const_string(1, "library");
    asm.const_string(3, "Doc");
    asm.invoke(
        "direct",
        m(DOC_BUILDER, "<init>", "V", &[STRING, STRING, STRING]),
        &[0, 1, 2, 3],
    );
    for (name, reg) in [("title", 9u16), ("body", 10), ("kind", 11)] {
        asm.const_string(1, name);
        asm.const_(4, 1);
        asm.new_array(5, 4, "[Ljava/lang/String;");
        asm.const_(4, 0);
        asm.aput_object(reg, 5, 4);
        asm.invoke(
            "virtual",
            m(DOC_BUILDER, "setPropertyString", DOC_BUILDER, &[STRING, "[Ljava/lang/String;"]),
            &[0, 1, 5],
        );
        asm.move_result(0, true);
    }
    asm.invoke("virtual", m(DOC_BUILDER, "build", DOC, &[]), &[0]);
    asm.move_result(0, true);
    asm.const_(1, 1);
    asm.new_array(2, 1, &format!("[{DOC}"));
    asm.const_(1, 0);
    asm.aput_object(0, 2, 1);
    asm.invoke(
        "virtual",
        m(PUT_BUILDER, "addGenericDocuments", PUT_BUILDER, &[&format!("[{DOC}")]),
        &[7, 2],
    );
    asm.move_result(0, true);
    asm.ret(0, "void");
}

/// `v15` is the context. One page of documents is enough for this library.
fn emit_documents(asm: &mut Asm, rt: &str, index_desc: &str) {
    asm.invoke("static", m(rt, "db", DB, &[CONTEXT]), &[15]);
    asm.move_result(7, true);
    asm.invoke("static", m(rt, "ready", "V", &[DB]), &[7]);
    asm.new_instance(1, PUT_BUILDER);
    asm.invoke("direct", m(PUT_BUILDER, "<init>", "V", &[]), &[1]);
    asm.const_string(2, "SELECT CAST(rowid AS TEXT), title, body, kind FROM docs");
    asm.const_(0, 0);
    asm.invoke(
        "virtual",
        m(DB, "rawQuery", CURSOR, &[STRING, "[Ljava/lang/String;"]),
        &[7, 2, 0],
    );
    asm.move_result(0, true);
    asm.const_string(2, "");
    asm.const_string(3, "");
    asm.const_string(4, "");
    asm.const_string(5, "");
    asm.const_string(6, "");
    asm.const_(8, 0);
    asm.label("docs");
    asm.invoke("interface", m(CURSOR, "moveToNext", "Z", &[]), &[0]);
    asm.move_result(8, false);
    asm.if_zero("eqz", 8, "docs_done");
    asm.const_(8, 0);
    asm.invoke("interface", m(CURSOR, "getString", STRING, &["I"]), &[0, 8]);
    asm.move_result(3, true);
    asm.const_(8, 1);
    asm.invoke("interface", m(CURSOR, "getString", STRING, &["I"]), &[0, 8]);
    asm.move_result(4, true);
    asm.const_(8, 2);
    asm.invoke("interface", m(CURSOR, "getString", STRING, &["I"]), &[0, 8]);
    asm.move_result(5, true);
    asm.const_(8, 3);
    asm.invoke("interface", m(CURSOR, "getString", STRING, &["I"]), &[0, 8]);
    asm.move_result(6, true);
    asm.invoke(
        "static",
        m(index_desc, "addDoc", "V", &[PUT_BUILDER, STRING, STRING, STRING, STRING]),
        &[1, 3, 4, 5, 6],
    );
    asm.goto("docs");
    asm.label("docs_done");
    asm.invoke("interface", m(CURSOR, "close", "V", &[]), &[0]);
    asm.invoke("virtual", m(DB, "close", "V", &[]), &[7]);
    asm.invoke("virtual", m(PUT_BUILDER, "build", PUT, &[]), &[1]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

fn emit_spec(asm: &mut Asm) {
    asm.new_instance(0, SPEC_BUILDER);
    asm.invoke("direct", m(SPEC_BUILDER, "<init>", "V", &[]), &[0]);
    asm.const_(1, 2);
    asm.invoke(
        "virtual",
        m(SPEC_BUILDER, "setTermMatch", SPEC_BUILDER, &["I"]),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.const_string(1, "Doc");
    asm.const_(2, 1);
    asm.new_array(3, 2, "[Ljava/lang/String;");
    asm.const_(2, 0);
    asm.aput_object(1, 3, 2);
    asm.invoke(
        "virtual",
        m(SPEC_BUILDER, "addFilterSchemas", SPEC_BUILDER, &["[Ljava/lang/String;"]),
        &[0, 3],
    );
    asm.move_result(0, true);
    asm.const_(1, 10000);
    asm.invoke(
        "virtual",
        m(SPEC_BUILDER, "setSnippetCount", SPEC_BUILDER, &["I"]),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.invoke(
        "virtual",
        m(SPEC_BUILDER, "setSnippetCountPerProperty", SPEC_BUILDER, &["I"]),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.invoke(
        "virtual",
        m(SPEC_BUILDER, "setMaxSnippetSize", SPEC_BUILDER, &["I"]),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.invoke(
        "virtual",
        m(SPEC_BUILDER, "setResultCountPerPage", SPEC_BUILDER, &["I"]),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.invoke("virtual", m(SPEC_BUILDER, "build", SPEC, &[]), &[0]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

fn emit_bare(asm: &mut Asm) {
    asm.new_instance(4, BUILDER);
    asm.invoke("direct", m(BUILDER, "<init>", "V", &[]), &[4]);
    asm.invoke("virtual", append(), &[4, 13]);
    asm.move_result(2, true);
    asm.const_string(6, "  ·  ");
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(2, true);
    asm.invoke("virtual", append(), &[4, 12]);
    asm.move_result(2, true);
    asm.invoke("virtual", m(BUILDER, "toString", STRING, &[]), &[4]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v15` is one search hit. Each match is its own row.
fn emit_line(asm: &mut Asm, rt: &str) {
    asm.if_zero("nez", 15, "line_hit");
    asm.const_string(0, "");
    asm.ret(0, "obj");
    asm.label("line_hit");
    asm.invoke("virtual", m(HIT, "getGenericDocument", DOC, &[]), &[15]);
    asm.move_result(14, true);
    asm.if_zero("nez", 14, "line_doc");
    asm.const_string(0, "");
    asm.ret(0, "obj");
    asm.label("line_doc");
    asm.const_string(0, "title");
    asm.invoke("virtual", m(DOC, "getPropertyString", STRING, &[STRING]), &[14, 0]);
    asm.move_result(13, true);
    blank(asm, 13, "line_title");
    asm.const_string(0, "kind");
    asm.invoke("virtual", m(DOC, "getPropertyString", STRING, &[STRING]), &[14, 0]);
    asm.move_result(12, true);
    blank(asm, 12, "line_kind");
    asm.invoke("virtual", m(HIT, "getMatchInfos", "Ljava/util/List;", &[]), &[15]);
    asm.move_result(11, true);
    asm.if_zero("eqz", 11, "line_none");
    asm.invoke("interface", m("Ljava/util/List;", "size", "I", &[]), &[11]);
    asm.move_result(0, false);
    asm.const_(5, 0);
    asm.if_cmp("le", 0, 5, "line_none_hit");
    asm.new_instance(4, BUILDER);
    asm.invoke("direct", m(BUILDER, "<init>", "V", &[]), &[4]);
    asm.const_(2, 0);
    asm.const_(3, 0);
    asm.const_(5, 0);
    asm.const_(7, 0);
    asm.const_(8, 0);
    asm.const_(9, 0);
    asm.const_(10, 0);
    asm.const_string(1, "");
    asm.const_string(6, "");
    asm.label("line_loop");
    asm.if_cmp("ge", 2, 0, "line_done");
    asm.invoke("interface", m("Ljava/util/List;", "get", "Ljava/lang/Object;", &["I"]), &[11, 2]);
    asm.move_result(3, true);
    asm.if_zero("eqz", 3, "line_next");
    asm.check_cast(3, MATCH);
    asm.invoke("virtual", m(MATCH, "getPropertyPath", STRING, &[]), &[3]);
    asm.move_result(6, true);
    asm.if_zero("eqz", 6, "line_next");
    asm.invoke("virtual", m(DOC, "getPropertyString", STRING, &[STRING]), &[14, 6]);
    asm.move_result(7, true);
    asm.if_zero("eqz", 7, "line_next");
    asm.invoke("virtual", m(MATCH, "getExactMatchRange", RANGE, &[]), &[3]);
    asm.move_result(8, true);
    asm.if_zero("eqz", 8, "line_next");
    asm.invoke("virtual", m(RANGE, "getStart", "I", &[]), &[8]);
    asm.move_result(9, false);
    asm.invoke("virtual", m(RANGE, "getEnd", "I", &[]), &[8]);
    asm.move_result(10, false);
    asm.invoke("static", m(rt, "span", STRING, &[STRING, "I", "I"]), &[7, 9, 10]);
    asm.move_result(6, true);
    asm.if_zero("eqz", 6, "line_next");
    asm.invoke("virtual", length_of(STRING), &[6]);
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "line_next");
    asm.invoke("virtual", length_of(BUILDER), &[4]);
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "line_add");
    asm.const_string(1, "\n");
    asm.invoke("virtual", append(), &[4, 1]);
    asm.move_result(1, true);
    asm.label("line_add");
    asm.invoke("virtual", append(), &[4, 13]);
    asm.move_result(1, true);
    asm.const_string(1, "  ·  ");
    asm.invoke("virtual", append(), &[4, 1]);
    asm.move_result(1, true);
    asm.invoke("virtual", append(), &[4, 12]);
    asm.move_result(1, true);
    asm.const_string(1, "  —  ");
    asm.invoke("virtual", append(), &[4, 1]);
    asm.move_result(1, true);
    asm.invoke("virtual", append(), &[4, 6]);
    asm.move_result(1, true);
    asm.label("line_next");
    asm.const_(5, 1);
    asm.binop(0x90, 2, 2, 5, "add-int");
    asm.goto("line_loop");
    asm.label("line_done");
    asm.invoke("virtual", length_of(BUILDER), &[4]);
    asm.move_result(5, false);
    asm.if_zero("nez", 5, "line_out");
    emit_bare(asm);
    asm.label("line_out");
    asm.invoke("virtual", m(BUILDER, "toString", STRING, &[]), &[4]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("line_none");
    emit_bare(asm);
    asm.label("line_none_hit");
    emit_bare(asm);
}

/// `v15` is the page.
fn emit_lines(asm: &mut Asm, index_desc: &str) {
    asm.if_zero("nez", 15, "page_have");
    asm.const_string(0, "");
    asm.ret(0, "obj");
    asm.label("page_have");
    asm.invoke("interface", m("Ljava/util/List;", "size", "I", &[]), &[15]);
    asm.move_result(0, false);
    asm.new_instance(1, BUILDER);
    asm.invoke("direct", m(BUILDER, "<init>", "V", &[]), &[1]);
    asm.const_(2, 0);
    asm.const_string(7, "\n");
    asm.const_(3, 0);
    asm.const_(4, 0);
    asm.const_(5, 0);
    asm.const_(6, 0);
    asm.label("page");
    asm.if_cmp("ge", 2, 0, "page_done");
    asm.invoke(
        "interface",
        m("Ljava/util/List;", "get", "Ljava/lang/Object;", &["I"]),
        &[15, 2],
    );
    asm.move_result(3, true);
    asm.if_zero("eqz", 3, "page_next");
    asm.check_cast(3, HIT);
    asm.invoke("static", m(index_desc, "line", STRING, &[HIT]), &[3]);
    asm.move_result(4, true);
    asm.if_zero("eqz", 4, "page_next");
    asm.invoke("virtual", length_of(BUILDER), &[1]);
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "page_add");
    asm.invoke("virtual", append(), &[1, 7]);
    asm.move_result(6, true);
    asm.label("page_add");
    asm.invoke("virtual", append(), &[1, 4]);
    asm.move_result(6, true);
    asm.label("page_next");
    asm.const_(5, 1);
    asm.binop(0x90, 2, 2, 5, "add-int");
    asm.goto("page");
    asm.label("page_done");
    asm.invoke("virtual", m(BUILDER, "toString", STRING, &[]), &[1]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v14` context, `v15` raw query. Null means the session failed.
fn emit_query(asm: &mut Asm, cb: &str) {
    asm.const_string(0, "app_search");
    asm.invoke(
        "virtual",
        m(CONTEXT, "getSystemService", "Ljava/lang/Object;", &[STRING]),
        &[14, 0],
    );
    asm.move_result(0, true);
    asm.if_zero("nez", 0, "query_go");
    asm.const_(0, 0);
    asm.ret(0, "obj");
    asm.label("query_go");
    asm.check_cast(0, MANAGER);
    asm.invoke(
        "static",
        m(
            "Ljava/util/concurrent/Executors;",
            "newSingleThreadExecutor",
            "Ljava/util/concurrent/ExecutorService;",
            &[],
        ),
        &[],
    );
    asm.move_result(1, true);
    asm.const_(2, 1);
    asm.new_instance(3, "Ljava/util/concurrent/CountDownLatch;");
    asm.invoke(
        "direct",
        m("Ljava/util/concurrent/CountDownLatch;", "<init>", "V", &["I"]),
        &[3, 2],
    );
    asm.new_instance(4, cb);
    asm.invoke("direct", m(cb, "<init>", "V", &[]), &[4]);
    asm.iput_obj(3, 4, f(cb, "latch", "Ljava/util/concurrent/CountDownLatch;"));
    asm.iput_obj(1, 4, f(cb, "executor", "Ljava/util/concurrent/Executor;"));
    asm.iput_obj(15, 4, f(cb, "query", STRING));
    asm.iput_obj(14, 4, f(cb, "context", "Ljava/lang/Object;"));
    asm.const_(2, 0);
    asm.iput(2, 4, f(cb, "stage", "I"));
    asm.new_instance(5, SEARCH_CONTEXT_BUILDER);
    asm.const_string(6, "library");
    asm.invoke(
        "direct",
        m(SEARCH_CONTEXT_BUILDER, "<init>", "V", &[STRING]),
        &[5, 6],
    );
    asm.invoke("virtual", m(SEARCH_CONTEXT_BUILDER, "build", SEARCH_CONTEXT, &[]), &[5]);
    asm.move_result(5, true);
    asm.invoke(
        "virtual",
        m(
            MANAGER,
            "createSearchSession",
            "V",
            &[SEARCH_CONTEXT, "Ljava/util/concurrent/Executor;", CONSUMER],
        ),
        &[0, 5, 1, 4],
    );
    asm.const_wide(6, 8);
    asm.const_string(8, "SECONDS");
    asm.invoke(
        "static",
        m(
            "Ljava/util/concurrent/TimeUnit;",
            "valueOf",
            "Ljava/util/concurrent/TimeUnit;",
            &[STRING],
        ),
        &[8],
    );
    asm.move_result(8, true);
    asm.invoke(
        "virtual",
        m(
            "Ljava/util/concurrent/CountDownLatch;",
            "await",
            "Z",
            &["J", "Ljava/util/concurrent/TimeUnit;"],
        ),
        &[3, 6, 7, 8],
    );
    asm.move_result(9, false);
    asm.check_cast(1, "Ljava/util/concurrent/ExecutorService;");
    asm.invoke(
        "interface",
        m("Ljava/util/concurrent/ExecutorService;", "shutdown", "V", &[]),
        &[1],
    );
    asm.iget_obj(0, 4, f(cb, "text", STRING));
    asm.ret(0, "obj");
}

fn emit_finish(asm: &mut Asm, cb: &str) {
    asm.iget_obj(0, 5, f(cb, "hits", "Ljava/lang/Object;"));
    asm.if_zero("eqz", 0, "finish_session");
    asm.check_cast(0, HITS);
    asm.invoke("virtual", m(HITS, "close", "V", &[]), &[0]);
    asm.label("finish_session");
    asm.iget_obj(0, 5, f(cb, "session", "Ljava/lang/Object;"));
    asm.if_zero("eqz", 0, "finish_latch");
    asm.check_cast(0, SESSION);
    asm.invoke("virtual", m(SESSION, "close", "V", &[]), &[0]);
    asm.label("finish_latch");
    asm.iget_obj(0, 5, f(cb, "latch", "Ljava/util/concurrent/CountDownLatch;"));
    asm.invoke(
        "virtual",
        m("Ljava/util/concurrent/CountDownLatch;", "countDown", "V", &[]),
        &[0],
    );
    asm.ret(0, "void");
}

fn emit_stop(asm: &mut Asm, cb: &str) {
    asm.invoke("virtual", m(cb, "finish", "V", &[]), &[14]);
    asm.ret(0, "void");
}

fn emit_accept(asm: &mut Asm, index_desc: &str, cb: &str) {
    asm.if_zero("nez", 15, "accept_value");
    emit_stop(asm, cb);
    asm.label("accept_value");
    asm.check_cast(15, RESULT);
    asm.invoke("virtual", m(RESULT, "isSuccess", "Z", &[]), &[15]);
    asm.move_result(0, false);
    asm.if_zero("nez", 0, "accept_ok");
    emit_stop(asm, cb);
    asm.label("accept_ok");
    asm.invoke("virtual", m(RESULT, "getResultValue", "Ljava/lang/Object;", &[]), &[15]);
    asm.move_result(1, true);
    asm.if_zero("nez", 1, "accept_payload");
    emit_stop(asm, cb);
    asm.label("accept_payload");
    asm.iget(0, 14, f(cb, "stage", "I"));
    asm.if_zero("nez", 0, "accept_later");
    asm.iput_obj(1, 14, f(cb, "session", "Ljava/lang/Object;"));
    asm.const_(0, 1);
    asm.iput(0, 14, f(cb, "stage", "I"));
    asm.invoke("static", m(index_desc, "schema", SET_SCHEMA, &[]), &[]);
    asm.move_result(2, true);
    asm.check_cast(1, SESSION);
    asm.iget_obj(3, 14, f(cb, "executor", "Ljava/util/concurrent/Executor;"));
    asm.invoke(
        "virtual",
        m(
            SESSION,
            "setSchema",
            "V",
            &[
                SET_SCHEMA,
                "Ljava/util/concurrent/Executor;",
                "Ljava/util/concurrent/Executor;",
                CONSUMER,
            ],
        ),
        &[1, 2, 3, 3, 14],
    );
    asm.ret(0, "void");
    asm.label("accept_later");
    asm.const_(2, 1);
    asm.if_cmp("ne", 0, 2, "accept_page");
    asm.iget_obj(4, 14, f(cb, "context", "Ljava/lang/Object;"));
    asm.check_cast(4, CONTEXT);
    asm.invoke("static", m(index_desc, "documents", PUT, &[CONTEXT]), &[4]);
    asm.move_result(2, true);
    asm.iget_obj(1, 14, f(cb, "session", "Ljava/lang/Object;"));
    asm.check_cast(1, SESSION);
    asm.iget_obj(3, 14, f(cb, "executor", "Ljava/util/concurrent/Executor;"));
    asm.invoke(
        "virtual",
        m(SESSION, "put", "V", &[PUT, "Ljava/util/concurrent/Executor;", BATCH_CB]),
        &[1, 2, 3, 14],
    );
    asm.ret(0, "void");
    asm.label("accept_page");
    asm.check_cast(1, "Ljava/util/List;");
    asm.const_string(4, "");
    asm.invoke("interface", m("Ljava/util/List;", "size", "I", &[]), &[1]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "accept_stop");
    asm.invoke(
        "static",
        m(index_desc, "lines", STRING, &["Ljava/util/List;"]),
        &[1],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, "accept_more");
    asm.invoke("virtual", length_of(STRING), &[2]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "accept_more");
    asm.iget_obj(3, 14, f(cb, "text", STRING));
    asm.if_zero("eqz", 3, "accept_store");
    asm.invoke("virtual", length_of(STRING), &[3]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "accept_store");
    asm.const_string(4, "\n");
    asm.invoke("virtual", m(STRING, "concat", STRING, &[STRING]), &[3, 4]);
    asm.move_result(3, true);
    asm.invoke("virtual", m(STRING, "concat", STRING, &[STRING]), &[3, 2]);
    asm.move_result(2, true);
    asm.label("accept_store");
    asm.iput_obj(2, 14, f(cb, "text", STRING));
    asm.label("accept_more");
    asm.iget_obj(1, 14, f(cb, "hits", "Ljava/lang/Object;"));
    asm.check_cast(1, HITS);
    asm.iget_obj(3, 14, f(cb, "executor", "Ljava/util/concurrent/Executor;"));
    asm.invoke(
        "virtual",
        m(HITS, "getNextPage", "V", &["Ljava/util/concurrent/Executor;", CONSUMER]),
        &[1, 3, 14],
    );
    asm.ret(0, "void");
    asm.label("accept_stop");
    emit_stop(asm, cb);
}

fn emit_on_result(asm: &mut Asm, index_desc: &str, cb: &str) {
    asm.if_zero("nez", 11, "put_ok");
    asm.invoke("virtual", m(cb, "finish", "V", &[]), &[10]);
    asm.ret(0, "void");
    asm.label("put_ok");
    asm.check_cast(11, BATCH);
    asm.invoke("virtual", m(BATCH, "isSuccess", "Z", &[]), &[11]);
    asm.move_result(0, false);
    asm.if_zero("nez", 0, "put_search");
    asm.invoke("virtual", m(cb, "finish", "V", &[]), &[10]);
    asm.ret(0, "void");
    asm.label("put_search");
    asm.invoke("static", m(index_desc, "spec", SPEC, &[]), &[]);
    asm.move_result(2, true);
    asm.iget_obj(0, 10, f(cb, "query", STRING));
    asm.iget_obj(1, 10, f(cb, "session", "Ljava/lang/Object;"));
    asm.check_cast(1, SESSION);
    asm.invoke(
        "virtual",
        m(SESSION, "search", HITS, &[STRING, SPEC]),
        &[1, 0, 2],
    );
    asm.move_result(2, true);
    asm.iput_obj(2, 10, f(cb, "hits", "Ljava/lang/Object;"));
    asm.const_(0, 2);
    asm.iput(0, 10, f(cb, "stage", "I"));
    asm.iget_obj(3, 10, f(cb, "executor", "Ljava/util/concurrent/Executor;"));
    asm.invoke(
        "virtual",
        m(HITS, "getNextPage", "V", &["Ljava/util/concurrent/Executor;", CONSUMER]),
        &[2, 3, 10],
    );
    asm.ret(0, "void");
}

fn emit_on_error(asm: &mut Asm, cb: &str) {
    asm.invoke("virtual", m(cb, "finish", "V", &[]), &[2]);
    asm.ret(0, "void");
}
