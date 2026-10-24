//! `Search` reads AppSearch when the device has it, and a built-in note list otherwise.
//!
//! AppSearch is API 31. `DroideRt.search` checks `SDK_INT` and `getSystemService("app_search")`
//! and only then calls `DroideSearch`. That class is the only one whose methods name
//! `android.app.appsearch`, so an older device never resolves those types. A null result,
//! a timeout, or an empty query uses the same five notes. An empty query does not call
//! AppSearch: the platform rejects it, and the full list is what the form wants.

use crate::dex::{
    Asm, ClassModel, DexFile, FieldRef, MethodModel, MethodRef, ACC_CONSTRUCTOR, ACC_PUBLIC,
    ACC_STATIC,
};

const NOTES: &[&str] = &[
    "Ink is a deep blue mixed on the swatch",
    "Clay is a warm red left on the shelf",
    "Leaf is the green of a fresh note",
    "Sand is a pale gold hour",
    "Paper holds the private note from the device",
];

const MANAGER: &str = "Landroid/app/appsearch/AppSearchManager;";
const CONTEXT: &str = "Landroid/app/appsearch/AppSearchManager$SearchContext;";
const CONTEXT_BUILDER: &str = "Landroid/app/appsearch/AppSearchManager$SearchContext$Builder;";
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
const BATCH: &str = "Landroid/app/appsearch/AppSearchBatchResult;";
const BATCH_CB: &str = "Landroid/app/appsearch/BatchResultCallback;";
const CONSUMER: &str = "Ljava/util/function/Consumer;";

pub fn attach(dex: &mut DexFile, package: &str, rt: &mut ClassModel) {
    let rt_desc = class_desc(package, "DroideRt");
    let search_desc = class_desc(package, "DroideSearch");
    let cb_desc = class_desc(package, "DroideSearch$Cb");

    let mut keep = Asm::new();
    emit_keep(&mut keep);
    rt.method(method(
        "keep",
        "V",
        vec![
            "Ljava/lang/StringBuilder;".into(),
            "Ljava/lang/String;".into(),
            "Ljava/lang/String;".into(),
        ],
        ACC_PUBLIC | ACC_STATIC,
        keep,
        8,
        3,
        2,
    ));

    let mut notes = Asm::new();
    emit_local_notes(&mut notes, &rt_desc);
    rt.method(method(
        "localNotes",
        "Ljava/lang/String;",
        vec!["Ljava/lang/String;".into()],
        ACC_PUBLIC | ACC_STATIC,
        notes,
        12,
        1,
        3,
    ));

    let mut search = Asm::new();
    emit_rt_search(&mut search, &rt_desc, &search_desc);
    rt.method(method(
        "search",
        "Ljava/lang/String;",
        vec![
            "Landroid/content/Context;".into(),
            "Ljava/lang/String;".into(),
        ],
        ACC_PUBLIC | ACC_STATIC,
        search,
        16,
        2,
        2,
    ));

    let mut api = ClassModel::new(&search_desc, "Ljava/lang/Object;", ACC_PUBLIC, "DroideSearch");
    let mut schema = Asm::new();
    emit_schema(&mut schema);
    api.method(method(
        "schema",
        SET_SCHEMA,
        Vec::new(),
        ACC_PUBLIC | ACC_STATIC,
        schema,
        8,
        0,
        2,
    ));
    let mut one = Asm::new();
    emit_note(&mut one);
    api.method(method(
        "note",
        "V",
        vec![
            PUT_BUILDER.into(),
            "Ljava/lang/String;".into(),
            "Ljava/lang/String;".into(),
        ],
        ACC_PUBLIC | ACC_STATIC,
        one,
        8,
        3,
        4,
    ));
    let mut docs = Asm::new();
    emit_documents(&mut docs, &search_desc);
    api.method(method(
        "documents",
        PUT,
        Vec::new(),
        ACC_PUBLIC | ACC_STATIC,
        docs,
        8,
        0,
        3,
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
    let mut lines = Asm::new();
    emit_lines(&mut lines);
    api.method(method(
        "lines",
        "Ljava/lang/String;",
        vec!["Ljava/util/List;".into()],
        ACC_PUBLIC | ACC_STATIC,
        lines,
        16,
        1,
        2,
    ));
    let mut query = Asm::new();
    emit_query(&mut query, &search_desc, &cb_desc);
    api.method(method(
        "query",
        "Ljava/lang/String;",
        vec![
            "Landroid/content/Context;".into(),
            "Ljava/lang/String;".into(),
        ],
        ACC_PUBLIC | ACC_STATIC,
        query,
        16,
        2,
        4,
    ));
    dex.add(api);

    let mut cb = ClassModel::new(&cb_desc, "Ljava/lang/Object;", ACC_PUBLIC, "DroideSearch");
    cb.interfaces().push(CONSUMER.into());
    cb.interfaces().push(BATCH_CB.into());
    cb.field("latch", "Ljava/util/concurrent/CountDownLatch;", ACC_PUBLIC);
    cb.field("executor", "Ljava/util/concurrent/Executor;", ACC_PUBLIC);
    cb.field("query", "Ljava/lang/String;", ACC_PUBLIC);
    cb.field("stage", "I", ACC_PUBLIC);
    cb.field("session", "Ljava/lang/Object;", ACC_PUBLIC);
    cb.field("text", "Ljava/lang/String;", ACC_PUBLIC);

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
    cb.method(method(
        "finish",
        "V",
        Vec::new(),
        ACC_PUBLIC,
        finish,
        6,
        1,
        1,
    ));

    let mut accept = Asm::new();
    emit_accept(&mut accept, &search_desc, &cb_desc);
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
    emit_on_result(&mut on_result, &search_desc, &cb_desc);
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
    on_error.invoke("virtual", m(&cb_desc, "finish", "V", &[]), &[2]);
    on_error.ret(0, "void");
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

fn string_builder_append() -> MethodRef {
    m(
        "Ljava/lang/StringBuilder;",
        "append",
        "Ljava/lang/StringBuilder;",
        &["Ljava/lang/String;"],
    )
}

fn string_length() -> MethodRef {
    m("Ljava/lang/String;", "length", "I", &[])
}

/// Append `v6` to builder `v5`. A newline comes first when the builder is not empty.
/// Each path returns, so a reference does not join an unset register.
fn emit_keep_line(asm: &mut Asm, tag: &str) {
    let plain = format!("plain_{tag}");
    asm.invoke("virtual", m("Ljava/lang/StringBuilder;", "length", "I", &[]), &[5]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, &plain);
    asm.const_string(1, "\n");
    asm.invoke("virtual", string_builder_append(), &[5, 1]);
    asm.move_result(2, true);
    asm.invoke("virtual", string_builder_append(), &[5, 6]);
    asm.move_result(2, true);
    asm.ret(0, "void");
    asm.label(&plain);
    asm.invoke("virtual", string_builder_append(), &[5, 6]);
    asm.move_result(2, true);
    asm.ret(0, "void");
}

/// `v5` builder, `v6` line, `v7` needle. An empty needle keeps every line.
fn emit_keep(asm: &mut Asm) {
    asm.invoke("virtual", string_length(), &[7]);
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "keep_all");
    asm.new_instance(1, "Ljava/util/Locale;");
    asm.const_string(2, "en");
    asm.invoke(
        "direct",
        m("Ljava/util/Locale;", "<init>", "V", &["Ljava/lang/String;"]),
        &[1, 2],
    );
    asm.invoke(
        "virtual",
        m(
            "Ljava/lang/String;",
            "toLowerCase",
            "Ljava/lang/String;",
            &["Ljava/util/Locale;"],
        ),
        &[6, 1],
    );
    asm.move_result(2, true);
    asm.invoke(
        "virtual",
        m(
            "Ljava/lang/String;",
            "contains",
            "Z",
            &["Ljava/lang/CharSequence;"],
        ),
        &[2, 7],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "keep_skip");
    emit_keep_line(asm, "hit");
    asm.label("keep_all");
    emit_keep_line(asm, "all");
    asm.label("keep_skip");
    asm.ret(0, "void");
}

/// `v11` is the query. A null query lists every note.
fn emit_local_notes(asm: &mut Asm, rt: &str) {
    asm.if_zero("nez", 11, "notes_have");
    asm.const_string(11, "");
    asm.label("notes_have");
    asm.new_instance(1, "Ljava/util/Locale;");
    asm.const_string(2, "en");
    asm.invoke(
        "direct",
        m("Ljava/util/Locale;", "<init>", "V", &["Ljava/lang/String;"]),
        &[1, 2],
    );
    asm.invoke(
        "virtual",
        m(
            "Ljava/lang/String;",
            "toLowerCase",
            "Ljava/lang/String;",
            &["Ljava/util/Locale;"],
        ),
        &[11, 1],
    );
    asm.move_result(1, true);
    asm.new_instance(0, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        m("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[0],
    );
    for note in NOTES {
        asm.const_string(2, note);
        asm.invoke(
            "static",
            m(
                rt,
                "keep",
                "V",
                &[
                    "Ljava/lang/StringBuilder;",
                    "Ljava/lang/String;",
                    "Ljava/lang/String;",
                ],
            ),
            &[0, 2, 1],
        );
    }
    asm.invoke(
        "virtual",
        m("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[0],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v14` context, `v15` query. AppSearch types are not named in this method.
fn emit_rt_search(asm: &mut Asm, rt: &str, search_desc: &str) {
    asm.if_zero("nez", 15, "search_len");
    asm.const_(0, 0);
    asm.goto("search_local");
    asm.label("search_len");
    asm.invoke("virtual", string_length(), &[15]);
    asm.move_result(0, false);
    asm.if_zero("nez", 0, "search_sdk");
    asm.const_(0, 0);
    asm.goto("search_local");
    asm.label("search_sdk");
    asm.sget(0, f("Landroid/os/Build$VERSION;", "SDK_INT", "I"));
    asm.const_(1, 31);
    asm.if_cmp("ge", 0, 1, "search_api");
    asm.const_(0, 0);
    asm.goto("search_local");
    asm.label("search_api");
    asm.const_string(0, "app_search");
    asm.invoke(
        "virtual",
        m(
            "Landroid/content/Context;",
            "getSystemService",
            "Ljava/lang/Object;",
            &["Ljava/lang/String;"],
        ),
        &[14, 0],
    );
    asm.move_result(0, true);
    asm.if_zero("nez", 0, "search_service");
    asm.const_(0, 0);
    asm.goto("search_local");
    asm.label("search_service");
    asm.invoke(
        "static",
        m(
            search_desc,
            "query",
            "Ljava/lang/String;",
            &["Landroid/content/Context;", "Ljava/lang/String;"],
        ),
        &[14, 15],
    );
    asm.move_result(0, true);
    asm.if_zero("nez", 0, "search_hit");
    asm.const_(0, 0);
    asm.goto("search_local");
    asm.label("search_hit");
    asm.ret(0, "obj");
    asm.label("search_local");
    asm.invoke(
        "static",
        m(rt, "localNotes", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[15],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

fn emit_schema(asm: &mut Asm) {
    asm.new_instance(0, STRING_PROP_BUILDER);
    asm.const_string(1, "body");
    asm.invoke(
        "direct",
        m(STRING_PROP_BUILDER, "<init>", "V", &["Ljava/lang/String;"]),
        &[0, 1],
    );
    // CARDINALITY_REQUIRED = 3, INDEXING_TYPE_PREFIXES = 2, TOKENIZER_TYPE_PLAIN = 1.
    asm.const_(1, 3);
    asm.invoke(
        "virtual",
        m(STRING_PROP_BUILDER, "setCardinality", STRING_PROP_BUILDER, &["I"]),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.const_(1, 2);
    asm.invoke(
        "virtual",
        m(STRING_PROP_BUILDER, "setIndexingType", STRING_PROP_BUILDER, &["I"]),
        &[0, 1],
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
    asm.move_result(1, true);
    asm.new_instance(0, SCHEMA_BUILDER);
    asm.const_string(2, "Note");
    asm.invoke(
        "direct",
        m(SCHEMA_BUILDER, "<init>", "V", &["Ljava/lang/String;"]),
        &[0, 2],
    );
    asm.invoke(
        "virtual",
        m(SCHEMA_BUILDER, "addProperty", SCHEMA_BUILDER, &[PROP]),
        &[0, 1],
    );
    asm.move_result(0, true);
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
        m(
            SET_SCHEMA_BUILDER,
            "addSchemas",
            SET_SCHEMA_BUILDER,
            &[&format!("[{SCHEMA}")],
        ),
        &[0, 3],
    );
    asm.move_result(0, true);
    asm.invoke("virtual", m(SET_SCHEMA_BUILDER, "build", SET_SCHEMA, &[]), &[0]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v5` request builder, `v6` id, `v7` body. Namespace `notes`, schema `Note`, property `body`.
fn emit_note(asm: &mut Asm) {
    asm.new_instance(0, DOC_BUILDER);
    asm.const_string(1, "notes");
    asm.const_string(2, "Note");
    asm.invoke(
        "direct",
        m(
            DOC_BUILDER,
            "<init>",
            "V",
            &["Ljava/lang/String;", "Ljava/lang/String;", "Ljava/lang/String;"],
        ),
        &[0, 1, 6, 2],
    );
    asm.const_string(1, "body");
    asm.const_(2, 1);
    asm.new_array(3, 2, "[Ljava/lang/String;");
    asm.const_(2, 0);
    asm.aput_object(7, 3, 2);
    asm.invoke(
        "virtual",
        m(
            DOC_BUILDER,
            "setPropertyString",
            DOC_BUILDER,
            &["Ljava/lang/String;", "[Ljava/lang/String;"],
        ),
        &[0, 1, 3],
    );
    asm.move_result(0, true);
    asm.invoke("virtual", m(DOC_BUILDER, "build", DOC, &[]), &[0]);
    asm.move_result(0, true);
    asm.const_(1, 1);
    asm.new_array(2, 1, &format!("[{DOC}"));
    asm.const_(1, 0);
    asm.aput_object(0, 2, 1);
    asm.invoke(
        "virtual",
        m(
            PUT_BUILDER,
            "addGenericDocuments",
            PUT_BUILDER,
            &[&format!("[{DOC}")],
        ),
        &[5, 2],
    );
    asm.move_result(0, true);
    asm.ret(0, "void");
}

fn emit_documents(asm: &mut Asm, search_desc: &str) {
    asm.new_instance(0, PUT_BUILDER);
    asm.invoke("direct", m(PUT_BUILDER, "<init>", "V", &[]), &[0]);
    let ids = ["ink", "clay", "leaf", "sand", "paper"];
    for (id, note) in ids.iter().zip(NOTES.iter()) {
        asm.const_string(1, id);
        asm.const_string(2, note);
        asm.invoke(
            "static",
            m(
                search_desc,
                "note",
                "V",
                &[PUT_BUILDER, "Ljava/lang/String;", "Ljava/lang/String;"],
            ),
            &[0, 1, 2],
        );
    }
    asm.invoke("virtual", m(PUT_BUILDER, "build", PUT, &[]), &[0]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

fn emit_spec(asm: &mut Asm) {
    asm.new_instance(0, SPEC_BUILDER);
    asm.invoke("direct", m(SPEC_BUILDER, "<init>", "V", &[]), &[0]);
    // TERM_MATCH_PREFIX = 2. A query of "ink" matches the token "ink".
    asm.const_(1, 2);
    asm.invoke(
        "virtual",
        m(SPEC_BUILDER, "setTermMatch", SPEC_BUILDER, &["I"]),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.const_string(1, "Note");
    asm.const_(2, 1);
    asm.new_array(3, 2, "[Ljava/lang/String;");
    asm.const_(2, 0);
    asm.aput_object(1, 3, 2);
    asm.invoke(
        "virtual",
        m(
            SPEC_BUILDER,
            "addFilterSchemas",
            SPEC_BUILDER,
            &["[Ljava/lang/String;"],
        ),
        &[0, 3],
    );
    asm.move_result(0, true);
    asm.invoke("virtual", m(SPEC_BUILDER, "build", SPEC, &[]), &[0]);
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v15` is the page. One page holds the five notes. A null list is an empty string.
fn emit_lines(asm: &mut Asm) {
    asm.if_zero("nez", 15, "rows_have");
    asm.const_string(0, "");
    asm.ret(0, "obj");
    asm.label("rows_have");
    asm.invoke("interface", m("Ljava/util/List;", "size", "I", &[]), &[15]);
    asm.move_result(0, false);
    asm.new_instance(1, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        m("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[1],
    );
    asm.const_(2, 0);
    asm.const_string(7, "\n");
    // const/4 0 joins the back edge, where these registers hold a reference or an int.
    asm.const_(3, 0);
    asm.const_(4, 0);
    asm.const_(5, 0);
    asm.const_(6, 0);
    asm.label("rows");
    asm.if_cmp("ge", 2, 0, "rows_done");
    asm.invoke(
        "interface",
        m("Ljava/util/List;", "get", "Ljava/lang/Object;", &["I"]),
        &[15, 2],
    );
    asm.move_result(3, true);
    asm.if_zero("eqz", 3, "rows_next");
    asm.check_cast(3, HIT);
    asm.invoke("virtual", m(HIT, "getGenericDocument", DOC, &[]), &[3]);
    asm.move_result(3, true);
    asm.if_zero("eqz", 3, "rows_next");
    asm.const_string(4, "body");
    asm.invoke(
        "virtual",
        m(DOC, "getPropertyString", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[3, 4],
    );
    asm.move_result(4, true);
    asm.if_zero("eqz", 4, "rows_next");
    asm.invoke("virtual", m("Ljava/lang/StringBuilder;", "length", "I", &[]), &[1]);
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "rows_add");
    asm.invoke("virtual", string_builder_append(), &[1, 7]);
    asm.move_result(6, true);
    asm.label("rows_add");
    asm.invoke("virtual", string_builder_append(), &[1, 4]);
    asm.move_result(6, true);
    asm.label("rows_next");
    asm.const_(5, 1);
    asm.binop(0x90, 2, 2, 5, "add-int");
    asm.goto("rows");
    asm.label("rows_done");
    asm.invoke(
        "virtual",
        m("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[1],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// `v14` context, `v15` query. The caller already checked the API. Null means the session failed.
fn emit_query(asm: &mut Asm, search_desc: &str, cb: &str) {
    asm.const_string(0, "app_search");
    asm.invoke(
        "virtual",
        m(
            "Landroid/content/Context;",
            "getSystemService",
            "Ljava/lang/Object;",
            &["Ljava/lang/String;"],
        ),
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
    asm.iput_obj(15, 4, f(cb, "query", "Ljava/lang/String;"));
    asm.const_(2, 0);
    asm.iput(2, 4, f(cb, "stage", "I"));
    asm.new_instance(5, CONTEXT_BUILDER);
    asm.const_string(6, "notes");
    asm.invoke(
        "direct",
        m(CONTEXT_BUILDER, "<init>", "V", &["Ljava/lang/String;"]),
        &[5, 6],
    );
    asm.invoke("virtual", m(CONTEXT_BUILDER, "build", CONTEXT, &[]), &[5]);
    asm.move_result(5, true);
    asm.invoke(
        "virtual",
        m(MANAGER, "createSearchSession", "V", &[CONTEXT, "Ljava/util/concurrent/Executor;", CONSUMER]),
        &[0, 5, 1, 4],
    );
    // Eight seconds. v6 and v7 are the wide pair, and the callback thread is not this one.
    asm.const_wide(6, 8);
    asm.const_string(8, "SECONDS");
    asm.invoke(
        "static",
        m(
            "Ljava/util/concurrent/TimeUnit;",
            "valueOf",
            "Ljava/util/concurrent/TimeUnit;",
            &["Ljava/lang/String;"],
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
    asm.iget_obj(0, 4, f(cb, "text", "Ljava/lang/String;"));
    let _ = search_desc;
    asm.ret(0, "obj");
}

/// `v5` is the callback. Close the session when one was opened, then release the latch.
fn emit_finish(asm: &mut Asm, cb: &str) {
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

/// `v14` callback, `v15` AppSearchResult. Stage 0 opens the schema, stage 1 puts notes, stage 2 reads the page.
fn emit_accept(asm: &mut Asm, search_desc: &str, cb: &str) {
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
    asm.invoke("static", m(search_desc, "schema", SET_SCHEMA, &[]), &[]);
    asm.move_result(2, true);
    asm.check_cast(1, SESSION);
    asm.iget_obj(3, 14, f(cb, "executor", "Ljava/util/concurrent/Executor;"));
    asm.invoke(
        "virtual",
        m(
            SESSION,
            "setSchema",
            "V",
            &[SET_SCHEMA, "Ljava/util/concurrent/Executor;", "Ljava/util/concurrent/Executor;", CONSUMER],
        ),
        &[1, 2, 3, 3, 14],
    );
    asm.ret(0, "void");
    asm.label("accept_later");
    asm.const_(2, 1);
    asm.if_cmp("ne", 0, 2, "accept_page");
    asm.invoke("static", m(search_desc, "documents", PUT, &[]), &[]);
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
    asm.invoke(
        "static",
        m(search_desc, "lines", "Ljava/lang/String;", &["Ljava/util/List;"]),
        &[1],
    );
    asm.move_result(1, true);
    asm.iput_obj(1, 14, f(cb, "text", "Ljava/lang/String;"));
    emit_stop(asm, cb);
}

/// `v10` callback, `v11` batch result from put. A successful put searches and asks for one page.
fn emit_on_result(asm: &mut Asm, search_desc: &str, cb: &str) {
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
    asm.invoke("static", m(search_desc, "spec", SPEC, &[]), &[]);
    asm.move_result(2, true);
    asm.iget_obj(0, 10, f(cb, "query", "Ljava/lang/String;"));
    asm.iget_obj(1, 10, f(cb, "session", "Ljava/lang/Object;"));
    asm.check_cast(1, SESSION);
    asm.invoke(
        "virtual",
        m(SESSION, "search", HITS, &["Ljava/lang/String;", SPEC]),
        &[1, 0, 2],
    );
    asm.move_result(2, true);
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
