//! DEX code generation. Six scratch registers sit under the locals, then `this`
//! and the parameters, so the arguments occupy the last `ins` slots.
//! Floats are built with int-to-float. A const/4 zero is a join for both ints and refs.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::{Map, Value};

use crate::ast::*;
use crate::check::{Checked, FormChecked, RoutineInfo};
use crate::dex::{
    Asm, ClassModel, DexFile, FieldRef, MethodModel, MethodRef, ACC_CONSTRUCTOR, ACC_NATIVE,
    ACC_PUBLIC, ACC_STATIC,
};
use crate::error::CompileError;
use crate::index;
use crate::search;
use crate::widgets::MethodRef as WMethod;
use crate::widgets::{default_props, getter, parse_color, setter, WidgetSpec, WIDGETS};

const SCRATCH: u16 = 6;

pub struct CompiledDex {
    pub dex: DexFile,
    pub bytes: Vec<u8>,
}

pub fn compile_dex(package: &str, checked: &Checked) -> Result<CompiledDex, CompileError> {
    let mut compiler = Compiler::new(package);
    compiler.emit(checked)?;
    let bytes = compiler.dex.build()?;
    Ok(CompiledDex {
        dex: compiler.dex,
        bytes,
    })
}

struct Compiler {
    package: String,
    dex: DexFile,
    lab: u32,
    native_desc: String,
    rt_desc: String,
    /// TakePhoto and OpenDocument handlers in source order. Photo codes start
    /// at 1. Document codes start at 256. One handler keeps one code per kind.
    result_codes: Vec<ResultUse>,
}

#[derive(Clone)]
struct ResultUse {
    photo: bool,
    handler: String,
    code: i64,
}

struct Frame {
    order: Vec<String>,
    slots: HashMap<String, (u16, String)>,
    param_names: HashSet<String>,
    this: Option<u16>,
    registers: u16,
    ins: u16,
}

struct Gen<'a> {
    asm: &'a mut Asm,
    frame: &'a Frame,
    lab: &'a mut u32,
    owner: String,
    package: String,
    native_desc: String,
    result_codes: Vec<ResultUse>,
    sp: u16,
    outs: u16,
    ret_label: String,
}

type Val = (u16, String);

impl Compiler {
    fn new(package: &str) -> Self {
        Self {
            package: package.to_string(),
            dex: DexFile::new(),
            lab: 0,
            native_desc: class_desc(package, "Native"),
            rt_desc: class_desc(package, "DroideRt"),
            result_codes: Vec::new(),
        }
    }

    fn emit(&mut self, checked: &Checked) -> Result<(), CompileError> {
        self.emit_runtime();
        if !checked.natives.is_empty() {
            self.emit_natives(checked);
        }
        for unit in checked.units.values() {
            self.emit_unit(unit)?;
        }
        for form in checked.forms.values() {
            self.emit_form(form)?;
        }
        Ok(())
    }

    fn emit_runtime(&mut self) {
        let mut cls = ClassModel::new(&self.rt_desc, "Ljava/lang/Object;", ACC_PUBLIC, "DroideRt");
        let mut asm = Asm::new();
        // v6 = context, v7 = px. 0.5f is 1.0/2.0 so no raw float bits are loaded.
        asm.invoke(
            "virtual",
            method(
                "Landroid/content/Context;",
                "getResources",
                "Landroid/content/res/Resources;",
                &[],
            ),
            &[6],
        );
        asm.move_result(0, true);
        asm.invoke(
            "virtual",
            method(
                "Landroid/content/res/Resources;",
                "getDisplayMetrics",
                "Landroid/util/DisplayMetrics;",
                &[],
            ),
            &[0],
        );
        asm.move_result(0, true);
        asm.iget(1, 0, field("Landroid/util/DisplayMetrics;", "density", "F"));
        asm.unop(0x82, 2, 7, "int-to-float");
        asm.binop(0xA8, 2, 2, 1, "mul-float");
        asm.const_(3, 1);
        asm.unop(0x82, 3, 3, "int-to-float");
        asm.const_(4, 2);
        asm.unop(0x82, 4, 4, "int-to-float");
        asm.binop(0xA9, 3, 3, 4, "div-float");
        asm.binop(0xA6, 2, 2, 3, "add-float");
        asm.unop(0x87, 0, 2, "float-to-int");
        asm.ret(0, "int");
        cls.method(MethodModel::new(
            "dp",
            "I",
            vec!["Landroid/content/Context;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(asm),
            8,
            2,
            1,
        ));
        let mut grow = Asm::new();
        emit_grow(&mut grow);
        cls.method(MethodModel::new(
            "grow",
            "V",
            vec!["Landroid/view/View;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(grow),
            12,
            1,
            3,
        ));
        let mut place = Asm::new();
        emit_place(&mut place, &self.rt_desc);
        cls.method(MethodModel::new(
            "place",
            "V",
            vec!["Landroid/view/View;".into(), "I".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(place),
            8,
            3,
            2,
        ));
        let mut is_int = Asm::new();
        emit_is_int(&mut is_int);
        cls.method(MethodModel::new(
            "isInt",
            "Z",
            vec!["Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(is_int),
            5,
            1,
            2,
        ));
        let mut write_text = Asm::new();
        emit_write_text(&mut write_text);
        cls.method(MethodModel::new(
            "writeText",
            "V",
            vec![
                "Landroid/content/Context;".into(),
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(write_text),
            8,
            3,
            3,
        ));
        let mut read_text = Asm::new();
        emit_read_text(&mut read_text);
        cls.method(MethodModel::new(
            "readText",
            "Ljava/lang/String;",
            vec![
                "Landroid/content/Context;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(read_text),
            8,
            2,
            3,
        ));
        let mut where_loc = Asm::new();
        emit_where(&mut where_loc);
        cls.method(MethodModel::new(
            "where",
            "Ljava/lang/String;",
            vec!["Landroid/content/Context;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(where_loc),
            12,
            1,
            3,
        ));
        let mut camera = Asm::new();
        emit_open_camera(&mut camera);
        cls.method(MethodModel::new(
            "openCamera",
            "V",
            vec!["Landroid/content/Context;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(camera),
            6,
            1,
            3,
        ));
        let mut form_arg = Asm::new();
        emit_form_arg(&mut form_arg, "S");
        cls.method(MethodModel::new(
            "arg",
            "Ljava/lang/String;",
            vec!["Landroid/app/Activity;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(form_arg),
            8,
            2,
            3,
        ));
        let mut form_int = Asm::new();
        emit_form_arg(&mut form_int, "I");
        cls.method(MethodModel::new(
            "argInt",
            "I",
            vec!["Landroid/app/Activity;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(form_int),
            8,
            2,
            3,
        ));
        let mut form_bool = Asm::new();
        emit_form_arg(&mut form_bool, "B");
        cls.method(MethodModel::new(
            "argBool",
            "Z",
            vec!["Landroid/app/Activity;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(form_bool),
            8,
            2,
            3,
        ));
        let mut take_photo = Asm::new();
        emit_take_photo(&mut take_photo);
        cls.method(MethodModel::new(
            "takePhoto",
            "Z",
            vec!["Landroid/app/Activity;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(take_photo),
            8,
            2,
            3,
        ));
        let mut open_document = Asm::new();
        emit_open_document(&mut open_document);
        cls.method(MethodModel::new(
            "openDocument",
            "Z",
            vec!["Landroid/app/Activity;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(open_document),
            8,
            2,
            3,
        ));
        let mut photo_path = Asm::new();
        emit_photo_path(&mut photo_path);
        cls.method(MethodModel::new(
            "photoPath",
            "Ljava/lang/String;",
            vec![
                "Landroid/content/Context;".into(),
                "Landroid/content/Intent;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(photo_path),
            8,
            2,
            4,
        ));
        let mut document_path = Asm::new();
        emit_document_path(&mut document_path);
        cls.method(MethodModel::new(
            "documentPath",
            "Ljava/lang/String;",
            vec![
                "Landroid/content/Context;".into(),
                "Landroid/content/Intent;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(document_path),
            12,
            2,
            4,
        ));
        let mut share = Asm::new();
        emit_share(&mut share);
        cls.method(MethodModel::new(
            "share",
            "V",
            vec![
                "Landroid/content/Context;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(share),
            8,
            2,
            3,
        ));
        let mut copy_text = Asm::new();
        emit_copy_text(&mut copy_text);
        cls.method(MethodModel::new(
            "copyText",
            "V",
            vec![
                "Landroid/content/Context;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(copy_text),
            8,
            2,
            2,
        ));
        let mut line_count = Asm::new();
        emit_line_count(&mut line_count);
        cls.method(MethodModel::new(
            "lineCount",
            "I",
            vec!["Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(line_count),
            6,
            1,
            2,
        ));
        let mut toggle_line = Asm::new();
        emit_toggle_line(&mut toggle_line);
        cls.method(MethodModel::new(
            "toggleLine",
            "Ljava/lang/String;",
            vec!["Ljava/lang/String;".into(), "Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(toggle_line),
            12,
            2,
            2,
        ));
        let mut drop_names = Asm::new();
        emit_drop_names(&mut drop_names);
        cls.method(MethodModel::new(
            "dropNames",
            "Ljava/lang/String;",
            vec!["Ljava/lang/String;".into(), "Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(drop_names),
            14,
            2,
            2,
        ));
        let mut drop_parallel = Asm::new();
        emit_parallel(&mut drop_parallel, false);
        cls.method(MethodModel::new(
            "dropParallel",
            "Ljava/lang/String;",
            vec![
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(drop_parallel),
            12,
            3,
            2,
        ));
        let mut take_parallel = Asm::new();
        emit_parallel(&mut take_parallel, true);
        cls.method(MethodModel::new(
            "takeParallel",
            "Ljava/lang/String;",
            vec![
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(take_parallel),
            12,
            3,
            2,
        ));
        let mut join_notes = Asm::new();
        emit_join_notes(&mut join_notes, &self.rt_desc);
        cls.method(MethodModel::new(
            "joinNotes",
            "Ljava/lang/String;",
            vec![
                "Landroid/content/Context;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(join_notes),
            12,
            2,
            3,
        ));
        let mut same_days = Asm::new();
        emit_same_days(&mut same_days);
        cls.method(MethodModel::new(
            "sameDays",
            "Ljava/lang/String;",
            vec!["Ljava/lang/String;".into(), "Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(same_days),
            10,
            2,
            2,
        ));
        let mut age_day = Asm::new();
        emit_age_day(&mut age_day);
        cls.method(MethodModel::new(
            "ageDay",
            "Ljava/lang/String;",
            vec!["I".into(), "I".into(), "I".into(), "I".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(age_day),
            12,
            5,
            3,
        ));
        let mut clear_ages = Asm::new();
        emit_clear_ages(&mut clear_ages, &self.rt_desc);
        cls.method(MethodModel::new(
            "clearAges",
            "V",
            vec!["Landroid/content/Context;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(clear_ages),
            12,
            1,
            3,
        ));
        let mut place_age = Asm::new();
        emit_place_age(&mut place_age, &self.rt_desc);
        cls.method(MethodModel::new(
            "placeAge",
            "V",
            vec![
                "Landroid/content/Context;".into(),
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(place_age),
            16,
            4,
            3,
        ));
        let mut filter_lines = Asm::new();
        emit_filter_lines(&mut filter_lines);
        cls.method(MethodModel::new(
            "filterLines",
            "Ljava/lang/String;",
            vec!["Ljava/lang/String;".into(), "Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(filter_lines),
            10,
            2,
            2,
        ));
        let mut insert_line = Asm::new();
        emit_insert_line(&mut insert_line);
        cls.method(MethodModel::new(
            "insertLine",
            "Ljava/lang/String;",
            vec!["Ljava/lang/String;".into(), "Ljava/lang/String;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(insert_line),
            10,
            3,
            2,
        ));
        let mut date_key = Asm::new();
        emit_date_key(&mut date_key);
        cls.method(MethodModel::new(
            "dateKey",
            "I",
            vec!["Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(date_key),
            8,
            1,
            3,
        ));
        let mut date_part = Asm::new();
        emit_date_part(&mut date_part, &self.rt_desc);
        cls.method(MethodModel::new(
            "datePart",
            "I",
            vec!["Ljava/lang/String;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(date_part),
            6,
            2,
            2,
        ));
        let mut weave = Asm::new();
        emit_weave(&mut weave, &self.rt_desc);
        cls.method(MethodModel::new(
            "weave",
            "Ljava/lang/String;",
            vec![
                "I".into(),
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(weave),
            16,
            5,
            2,
        ));
        let mut journal_items = Asm::new();
        emit_journal_items(&mut journal_items, &self.rt_desc);
        cls.method(MethodModel::new(
            "journalItems",
            "Ljava/lang/String;",
            vec!["Ljava/lang/String;".into(), "Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(journal_items),
            16,
            2,
            3,
        ));
        let mut journal_details = Asm::new();
        emit_journal_details(&mut journal_details, &self.rt_desc);
        cls.method(MethodModel::new(
            "journalDetails",
            "Ljava/lang/String;",
            vec![
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(journal_details),
            16,
            3,
            3,
        ));
        let mut day_heading = Asm::new();
        emit_day_heading(&mut day_heading, &self.rt_desc);
        cls.method(MethodModel::new(
            "dayHeading",
            "Ljava/lang/String;",
            vec![
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(day_heading),
            12,
            3,
            3,
        ));
        let mut line_index = Asm::new();
        emit_line_index(&mut line_index);
        cls.method(MethodModel::new(
            "lineIndex",
            "I",
            vec!["Ljava/lang/String;".into(), "Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(line_index),
            8,
            2,
            2,
        ));
        let mut line_at = Asm::new();
        emit_line_at(&mut line_at);
        cls.method(MethodModel::new(
            "lineAt",
            "Ljava/lang/String;",
            vec!["Ljava/lang/String;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(line_at),
            8,
            2,
            2,
        ));
        let mut replace_at = Asm::new();
        emit_replace_at(&mut replace_at);
        cls.method(MethodModel::new(
            "replaceAt",
            "Ljava/lang/String;",
            vec![
                "Ljava/lang/String;".into(),
                "I".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(replace_at),
            10,
            3,
            2,
        ));
        let mut drop_at = Asm::new();
        emit_drop_at(&mut drop_at);
        cls.method(MethodModel::new(
            "dropAt",
            "Ljava/lang/String;",
            vec!["Ljava/lang/String;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(drop_at),
            8,
            2,
            2,
        ));
        let mut today = Asm::new();
        emit_stamp(&mut today, "yyyy MMMM d");
        cls.method(MethodModel::new(
            "today",
            "Ljava/lang/String;",
            vec!["Landroid/content/Context;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(today),
            8,
            1,
            3,
        ));
        let mut clock = Asm::new();
        emit_stamp(&mut clock, "HH:mm");
        cls.method(MethodModel::new(
            "time",
            "Ljava/lang/String;",
            vec!["Landroid/content/Context;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(clock),
            8,
            1,
            3,
        ));
        let mut paste_text = Asm::new();
        emit_paste_text(&mut paste_text);
        cls.method(MethodModel::new(
            "pasteText",
            "Ljava/lang/String;",
            vec!["Landroid/content/Context;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(paste_text),
            8,
            1,
            2,
        ));
        let rows_desc = class_desc(&self.package, "DroideRows");
        let mut lines = Asm::new();
        emit_set_lines(&mut lines, &rows_desc);
        cls.method(MethodModel::new(
            "setLines",
            "V",
            vec![
                "Landroid/widget/ListView;".into(),
                "Landroid/content/Context;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(lines),
            8,
            3,
            5,
        ));
        let mut details = Asm::new();
        emit_set_details(&mut details, &rows_desc);
        cls.method(MethodModel::new(
            "setDetails",
            "V",
            vec![
                "Landroid/widget/ListView;".into(),
                "Landroid/content/Context;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(details),
            8,
            3,
            5,
        ));
        let mut hug = Asm::new();
        emit_hug_scroll(&mut hug, &class_desc(&self.package, "DroideBar29"));
        cls.method(MethodModel::new(
            "hugScroll",
            "V",
            vec!["Landroid/view/View;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(hug),
            16,
            2,
            5,
        ));
        let mut paint = Asm::new();
        emit_row_paint(&mut paint, &rows_desc);
        cls.method(MethodModel::new(
            "paintRow",
            "V",
            vec!["Landroid/widget/ListView;".into(), "I".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(paint),
            8,
            3,
            2,
        ));
        let mut task = Asm::new();
        emit_paint_task(&mut task);
        cls.method(MethodModel::new(
            "paintTask",
            "V",
            vec![
                "Landroid/view/View;".into(),
                "Ljava/lang/String;".into(),
                "I".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(task),
            8,
            3,
            3,
        ));
        let mut span = Asm::new();
        emit_paint_span(&mut span, &rows_desc, &class_desc(&self.package, "DroideRail"));
        cls.method(MethodModel::new(
            "paintSpan",
            "V",
            vec![
                "Landroid/view/View;".into(),
                rows_desc.clone(),
                "I".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(span),
            12,
            3,
            3,
        ));
        let mut line_has = Asm::new();
        emit_line_has(&mut line_has);
        cls.method(MethodModel::new(
            "lineHas",
            "I",
            vec!["Ljava/lang/String;".into(), "Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(line_has),
            8,
            2,
            2,
        ));
        let mut picked = Asm::new();
        emit_paint_picked(&mut picked, &rows_desc);
        cls.method(MethodModel::new(
            "paintPicked",
            "V",
            vec!["Landroid/widget/ListView;".into(), "Ljava/lang/String;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(picked),
            8,
            2,
            2,
        ));
        let mut height = Asm::new();
        emit_set_dp_height(&mut height, &self.rt_desc);
        cls.method(MethodModel::new(
            "setDpHeight",
            "V",
            vec![
                "Landroid/view/View;".into(),
                "Landroid/content/Context;".into(),
                "I".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(height),
            8,
            3,
            2,
        ));
        let mut recolor = Asm::new();
        emit_recolor(&mut recolor);
        cls.method(MethodModel::new(
            "recolor",
            "V",
            vec!["Landroid/view/View;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(recolor),
            8,
            2,
            2,
        ));
        let mut page = Asm::new();
        emit_page(&mut page, &self.rt_desc);
        cls.method(MethodModel::new(
            "page",
            "V",
            vec!["Landroid/app/Activity;".into(), "I".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(page),
            8,
            2,
            2,
        ));
        let mut items_of = Asm::new();
        emit_tagged_string(&mut items_of, TAG_ITEMS);
        cls.method(MethodModel::new(
            "itemsOf",
            "Ljava/lang/String;",
            vec!["Landroid/view/View;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(items_of),
            4,
            1,
            2,
        ));
        let mut details_of = Asm::new();
        emit_tagged_string(&mut details_of, TAG_DETAILS);
        cls.method(MethodModel::new(
            "detailsOf",
            "Ljava/lang/String;",
            vec!["Landroid/view/View;".into()],
            ACC_PUBLIC | ACC_STATIC,
            Some(details_of),
            4,
            1,
            2,
        ));
        let mut spinner = Asm::new();
        emit_set_spinner(&mut spinner);
        cls.method(MethodModel::new(
            "setSpinner",
            "V",
            vec![
                "Landroid/widget/Spinner;".into(),
                "Landroid/content/Context;".into(),
                "Ljava/lang/String;".into(),
            ],
            ACC_PUBLIC | ACC_STATIC,
            Some(spinner),
            8,
            3,
            4,
        ));
        let package = self.package.clone();
        search::attach(&mut self.dex, &package, &mut cls);
        index::attach(&mut self.dex, &package, &mut cls);
        self.dex.add(cls);
        emit_rows_class(&mut self.dex, &self.package);
        emit_list_scroll(&mut self.dex, &self.package);
        emit_scroll_thumb(&mut self.dex, &self.package);
        emit_inset_background(&mut self.dex, &self.package);
        emit_rail_mark(&mut self.dex, &self.package);
        emit_tick_class(&mut self.dex, &self.package);
    }

    fn emit_natives(&mut self, checked: &Checked) {
        let mut cls = ClassModel::new(
            &self.native_desc,
            "Ljava/lang/Object;",
            ACC_PUBLIC,
            "Native",
        );
        let mut libs = Vec::new();
        if checked.natives.values().any(|native| !native.ndk) {
            libs.push("droide");
        }
        if checked.natives.values().any(|native| native.ndk) {
            libs.push("droide_ndk");
        }
        let mut clinit = Asm::new();
        for lib in libs {
            clinit.const_string(0, lib);
            clinit.invoke(
                "static",
                method(
                    "Ljava/lang/System;",
                    "loadLibrary",
                    "V",
                    &["Ljava/lang/String;"],
                ),
                &[0],
            );
        }
        clinit.ret(0, "void");
        cls.method(MethodModel::new(
            "<clinit>",
            "V",
            Vec::new(),
            ACC_STATIC | ACC_CONSTRUCTOR,
            Some(clinit),
            1,
            0,
            1,
        ));
        for native in checked.natives.values() {
            let params = vec!["I".to_string(); native.params.len()];
            cls.method(MethodModel::new(
                &native.name,
                "I",
                params,
                ACC_PUBLIC | ACC_STATIC | ACC_NATIVE,
                None,
                0,
                0,
                0,
            ));
        }
        self.dex.add(cls);
    }

    fn emit_unit(&mut self, unit: &crate::check::UnitChecked) -> Result<(), CompileError> {
        if unit.routines.is_empty() {
            return Ok(());
        }
        let desc = class_desc(&self.package, &unit.name);
        let source = format!("{}.droid", unit.name);
        let mut cls = ClassModel::new(&desc, "Ljava/lang/Object;", ACC_PUBLIC, &source);
        for rec in by_index(&unit.routines) {
            cls.method(self.compile_routine(rec, &desc, true)?);
        }
        self.dex.add(cls);
        Ok(())
    }

    fn emit_form(&mut self, form: &FormChecked) -> Result<(), CompileError> {
        let desc = class_desc(&self.package, &form.name);
        let source = format!("{}.droid", form.name);
        let mut cls = ClassModel::new(&desc, "Landroid/app/Activity;", ACC_PUBLIC, &source);
        for widget in ordered_widgets(form) {
            let kind = comp_str(&widget.comp, "type")?;
            let spec = spec_exact(kind)?;
            let field_desc = component_desc(&self.package, &form.name, kind, &widget.name, spec.desc);
            cls.field(&widget.name, &field_desc, ACC_PUBLIC);
        }
        self.result_codes = result_uses(form);
        cls.method(activity_init());
        cls.method(self.on_create(form, &desc)?);
        for rec in by_index(&form.routines) {
            cls.method(self.compile_routine(rec, &desc, false)?);
        }
        if !self.result_codes.is_empty() {
            cls.method(self.on_activity_result(&desc));
        }
        self.dex.add(cls);
        for widget in ordered_widgets(form) {
            if let Some(handler) = event_of(&widget.comp, "OnClick") {
                self.dex.add(self.listener(form, &desc, widget, handler)?);
            }
            if let Some(handler) = event_of(&widget.comp, "OnItemClick") {
                self.dex.add(self.item_listener(form, &desc, widget, handler)?);
            }
            if let Some(handler) = event_of(&widget.comp, "OnLongClick") {
                self.dex.add(self.long_listener(form, &desc, widget, handler)?);
            }
            if let Some(handler) = event_of(&widget.comp, "OnItemLongClick") {
                self.dex.add(self.item_long_listener(form, &desc, widget, handler)?);
            }
            if let Some(handler) = event_of(&widget.comp, "OnItemSelected") {
                self.dex.add(self.select_listener(form, &desc, widget, handler)?);
            }
            if let Some(handler) = event_of(&widget.comp, "OnChange") {
                self.dex.add(self.change_listener(form, &desc, widget, handler)?);
            }
            if let Some(handler) = event_of(&widget.comp, "OnEnter") {
                self.dex.add(self.enter_listener(form, &desc, widget, handler)?);
            }
        }
        // The method that names the listener is already encoded. The class
        // can follow it; DEX refs are resolved when the file is built.
        for handler in ask_handlers(form) {
            self.dex.add(self.ask_listener(form, &desc, &handler)?);
        }
        for widget in ordered_widgets(form) {
            if comp_str(&widget.comp, "type")? == "Timer" {
                self.dex.add(self.timer_class(form, &desc, widget)?);
            }
        }
        Ok(())
    }

    fn compile_routine(
        &mut self,
        rec: &RoutineInfo,
        owner: &str,
        is_static: bool,
    ) -> Result<MethodModel, CompileError> {
        if rec.routine.name.to_lowercase() == "oncreate" {
            return Err(CompileError::new(
                format!("{} is reserved", rec.routine.name),
                rec.routine.line,
                rec.routine.col,
                "",
            ));
        }
        let params = paired_params(rec)?;
        let mut locals: Vec<(String, String)> = rec
            .routine
            .locals
            .iter()
            .map(|loc| (loc.name.clone(), canon_type(&loc.type_name)))
            .collect();
        if rec.routine.kind == "function" {
            locals.push(("Result".into(), rec.ret.clone().unwrap_or_default()));
        }
        let frame = build_frame(is_static, &params, &locals)?;
        let mut asm = Asm::new();
        let package = self.package.clone();
        let native_desc = self.native_desc.clone();
        let codes = self.result_codes.clone();
        let outs = {
            let mut gen = Gen::new(
                &mut asm,
                &frame,
                &mut self.lab,
                owner,
                &package,
                &native_desc,
                &codes,
            );
            gen.prologue();
            gen.emit_body(&rec.routine.body)?;
            gen.finish(rec.ret.as_deref());
            gen.outs
        };
        let ret = match &rec.ret {
            None => "V".to_string(),
            Some(ty) => type_desc(ty)?,
        };
        let param_descs = rec
            .params
            .iter()
            .map(|ty| type_desc(ty))
            .collect::<Result<Vec<_>, _>>()?;
        let access = ACC_PUBLIC | if is_static { ACC_STATIC } else { 0 };
        Ok(MethodModel::new(
            &rec.routine.name,
            &ret,
            param_descs,
            access,
            Some(asm),
            frame.registers,
            frame.ins,
            outs,
        ))
    }

    /// `this` is v4, the request code v5, the result code v6, and the intent v7.
    /// Each handler writes the path register on both sides of the call join.
    fn on_activity_result(&self, desc: &str) -> MethodModel {
        let mut asm = Asm::new();
        let this = 4u16;
        let request = 5u16;
        let result = 6u16;
        let intent = 7u16;
        let code = 0u16;
        let ok = 1u16;
        let path = 2u16;
        for (index, item) in self.result_codes.iter().enumerate() {
            let next = format!("n{index}");
            let empty = format!("e{index}");
            let call = format!("c{index}");
            asm.const_(code, item.code);
            asm.if_cmp("ne", request, code, &next);
            asm.const_(ok, -1);
            asm.if_cmp("ne", result, ok, &empty);
            let runtime = if item.photo { "photoPath" } else { "documentPath" };
            asm.invoke(
                "static",
                method(
                    &self.rt_desc,
                    runtime,
                    "Ljava/lang/String;",
                    &["Landroid/content/Context;", "Landroid/content/Intent;"],
                ),
                &[this, intent],
            );
            asm.move_result(path, true);
            asm.goto(&call);
            asm.label(&empty);
            asm.const_string(path, "");
            asm.label(&call);
            asm.invoke(
                "virtual",
                method(desc, &item.handler, "V", &["Ljava/lang/String;"]),
                &[this, path],
            );
            asm.ret(0, "void");
            asm.label(&next);
        }
        asm.ret(0, "void");
        MethodModel::new(
            "onActivityResult",
            "V",
            vec![
                "I".into(),
                "I".into(),
                "Landroid/content/Intent;".into(),
            ],
            ACC_PUBLIC,
            Some(asm),
            8,
            4,
            2,
        )
    }

    fn on_create(&self, form: &FormChecked, desc: &str) -> Result<MethodModel, CompileError> {
        let mut asm = Asm::new();
        let this = 6u16;
        let bundle = 7u16;
        // v0 stays the ScrollView and v1 the FrameLayout for the whole method.
        for reg in 0..SCRATCH {
            asm.const_(reg, 0);
        }
        asm.invoke(
            "super",
            method(
                "Landroid/app/Activity;",
                "onCreate",
                "V",
                &["Landroid/os/Bundle;"],
            ),
            &[this, bundle],
        );
        let bg = background_color(&form.form)?;
        asm.new_instance(0, "Landroid/widget/ScrollView;");
        asm.invoke(
            "direct",
            method(
                "Landroid/widget/ScrollView;",
                "<init>",
                "V",
                &["Landroid/content/Context;"],
            ),
            &[0, this],
        );
        asm.const_(2, bg);
        asm.invoke(
            "virtual",
            method("Landroid/view/View;", "setBackgroundColor", "V", &["I"]),
            &[0, 2],
        );
        // A short form grows to the window so a bottom-docked bar meets the nav.
        asm.const_(2, 1);
        asm.invoke(
            "virtual",
            method("Landroid/widget/ScrollView;", "setFillViewport", "V", &["Z"]),
            &[0, 2],
        );
        // The page bar sits on the window edge. -1 means do not grow the view.
        asm.const_(2, -1);
        asm.invoke(
            "static",
            method(
                &self.rt_desc,
                "hugScroll",
                "V",
                &["Landroid/view/View;", "I"],
            ),
            &[0, 2],
        );
        asm.new_instance(1, "Landroid/widget/FrameLayout;");
        asm.invoke(
            "direct",
            method(
                "Landroid/widget/FrameLayout;",
                "<init>",
                "V",
                &["Landroid/content/Context;"],
            ),
            &[1, this],
        );
        asm.const_(2, bg);
        asm.invoke(
            "virtual",
            method("Landroid/view/View;", "setBackgroundColor", "V", &["I"]),
            &[1, 2],
        );
        let comps = components(&form.form);
        let mut width = int_or(&form.form, "width", 360)?;
        let mut height = int_or(&form.form, "height", 640)?;
        for comp in comps {
            // A child's left and top are margins inside its panel, not form coordinates.
            if !comp_parent(comp).is_empty() {
                continue;
            }
            if comp.get("type").and_then(|v| v.as_str()) == Some("Timer") {
                continue;
            }
            width = width.max(comp_int(comp, "left")?.saturating_add(comp_int(comp, "width")?));
            height = height.max(comp_int(comp, "top")?.saturating_add(comp_int(comp, "height")?));
        }
        self.place_frame(&mut asm, this, width, height);
        for comp in comps {
            self.place_widget(&mut asm, form, desc, this, comp)?;
        }
        self.place_children(&mut asm, desc, this, comps)?;
        asm.invoke(
            "virtual",
            method(
                "Landroid/view/ViewGroup;",
                "addView",
                "V",
                &["Landroid/view/View;"],
            ),
            &[0, 1],
        );
        asm.invoke(
            "virtual",
            method(
                "Landroid/app/Activity;",
                "setContentView",
                "V",
                &["Landroid/view/View;"],
            ),
            &[this, 0],
        );
        if let Some(handler) = event_of(&form.form, "OnCreate") {
            let rec = form.routines.get(&handler.to_lowercase()).ok_or_else(|| {
                CompileError::bare(format!("OnCreate handler {handler} is not declared"))
            })?;
            let key = handler_key(desc, rec)?;
            if rec.params.is_empty() {
                asm.invoke("virtual", key, &[this]);
            } else {
                asm.invoke("virtual", key, &[this, 1]);
            }
        }
        asm.ret(0, "void");
        // ProgressBar's constructor takes four words. DatePicker.init takes five.
        Ok(MethodModel::new(
            "onCreate",
            "V",
            vec!["Landroid/os/Bundle;".into()],
            ACC_PUBLIC,
            Some(asm),
            8,
            2,
            5,
        ))
    }

    fn place_frame(&self, asm: &mut Asm, this: u16, width: i64, height: i64) {
        let _ = width;
        // The page is as wide as the window, so a right-edge scrollbar meets the screen.
        asm.const_(4, -1);
        self.emit_dp(asm, this, 5, height);
        asm.new_instance(3, "Landroid/widget/FrameLayout$LayoutParams;");
        asm.invoke(
            "direct",
            method(
                "Landroid/widget/FrameLayout$LayoutParams;",
                "<init>",
                "V",
                &["I", "I"],
            ),
            &[3, 4, 5],
        );
        asm.invoke(
            "virtual",
            method(
                "Landroid/view/View;",
                "setLayoutParams",
                "V",
                &["Landroid/view/ViewGroup$LayoutParams;"],
            ),
            &[1, 3],
        );
    }

    fn place_timer(
        &self,
        asm: &mut Asm,
        form: &FormChecked,
        desc: &str,
        this: u16,
        comp: &Value,
    ) -> Result<(), CompileError> {
        let comp_name = comp_str(comp, "name")?;
        let ldesc = timer_desc(&self.package, &form.name, comp_name);
        let tick = tick_desc(&self.package);
        let props = comp.get("props").and_then(|v| v.as_object());
        let interval = match props.and_then(|map| map.get("Interval")) {
            Some(value) => json_int(value)?,
            None => 1000,
        };
        let enabled = props
            .and_then(|map| map.get("Enabled"))
            .is_some_and(truthy);
        asm.new_instance(2, &ldesc);
        asm.invoke("direct", method(&ldesc, "<init>", "V", &[desc]), &[2, this]);
        asm.const_(3, interval);
        asm.invoke(
            "virtual",
            method(&tick, "setInterval", "V", &["I"]),
            &[2, 3],
        );
        asm.const_(3, if enabled { 1 } else { 0 });
        asm.invoke(
            "virtual",
            method(&tick, "setEnabled", "V", &["Z"]),
            &[2, 3],
        );
        asm.iput_obj(2, this, field(desc, comp_name, &ldesc));
        Ok(())
    }

    fn timer_class(
        &self,
        form: &FormChecked,
        form_desc: &str,
        widget: &WidgetRef,
    ) -> Result<ClassModel, CompileError> {
        let name = comp_str(&widget.comp, "name")?;
        let ldesc = timer_desc(&self.package, &form.name, name);
        let tick = tick_desc(&self.package);
        let source = format!("{}.droid", form.name);
        let mut cls = ClassModel::new(&ldesc, &tick, ACC_PUBLIC, &source);
        cls.field("this$0", form_desc, ACC_PUBLIC);
        let mut init = Asm::new();
        init.invoke("direct", method(&tick, "<init>", "V", &[]), &[1]);
        init.iput_obj(2, 1, field(&ldesc, "this$0", form_desc));
        init.ret(0, "void");
        cls.method(MethodModel::new(
            "<init>",
            "V",
            vec![form_desc.to_string()],
            ACC_PUBLIC | ACC_CONSTRUCTOR,
            Some(init),
            3,
            2,
            1,
        ));
        let mut run = Asm::new();
        // v5 is this. The handler can turn the timer off, so enabled is read again.
        run.iget(0, 5, field(&tick, "enabled", "Z"));
        run.if_zero("eqz", 0, "done");
        if let Some(handler) = event_of(&widget.comp, "OnTimer") {
            if let Some(rec) = form.routines.get(&handler.to_lowercase()) {
                let key = handler_key(form_desc, rec)?;
                run.iget_obj(1, 5, field(&ldesc, "this$0", form_desc));
                run.invoke("virtual", key, &[1]);
                run.iget(0, 5, field(&tick, "enabled", "Z"));
                run.if_zero("eqz", 0, "done");
            }
        }
        run.invoke("virtual", method(&tick, "arm", "V", &[]), &[5]);
        run.label("done");
        run.ret(0, "void");
        cls.method(MethodModel::new(
            "run",
            "V",
            Vec::new(),
            ACC_PUBLIC,
            Some(run),
            6,
            1,
            1,
        ));
        Ok(cls)
    }

    fn place_widget(
        &self,
        asm: &mut Asm,
        form: &FormChecked,
        desc: &str,
        this: u16,
        comp: &Value,
    ) -> Result<(), CompileError> {
        let kind = comp_str(comp, "type")?;
        let comp_name = comp_str(comp, "name")?;
        let spec = spec_exact(kind)?;
        let wdesc = spec.desc;
        if kind == "Timer" {
            return self.place_timer(asm, form, desc, this, comp);
        }
        asm.new_instance(2, wdesc);
        if kind == "Progress" {
            // Null AttributeSet plus android.R.attr.progressBarStyleHorizontal (0x01010078).
            asm.const_(4, 0);
            asm.const_(5, 0x01010078);
            asm.invoke(
                "direct",
                method(
                    wdesc,
                    "<init>",
                    "V",
                    &[
                        "Landroid/content/Context;",
                        "Landroid/util/AttributeSet;",
                        "I",
                    ],
                ),
                &[2, this, 4, 5],
            );
            asm.const_(4, 0);
            asm.invoke(
                "virtual",
                method(
                    "Landroid/widget/ProgressBar;",
                    "setIndeterminate",
                    "V",
                    &["Z"],
                ),
                &[2, 4],
            );
        } else if kind == "Busy" {
            // The one-argument constructor is the circular indicator.
            asm.invoke(
                "direct",
                method(wdesc, "<init>", "V", &["Landroid/content/Context;"]),
                &[2, this],
            );
            asm.const_(4, 1);
            asm.invoke(
                "virtual",
                method(
                    "Landroid/widget/ProgressBar;",
                    "setIndeterminate",
                    "V",
                    &["Z"],
                ),
                &[2, 4],
            );
        } else {
            asm.invoke(
                "direct",
                method(wdesc, "<init>", "V", &["Landroid/content/Context;"]),
                &[2, this],
            );
        }
        if kind == "Button" {
            asm.const_(4, 0);
            asm.invoke(
                "virtual",
                method("Landroid/widget/TextView;", "setAllCaps", "V", &["Z"]),
                &[2, 4],
            );
        }
        if matches!(kind, "Edit" | "Number" | "Password" | "Email" | "Phone") {
            asm.const_(4, 1);
            asm.invoke(
                "virtual",
                method("Landroid/widget/TextView;", "setSingleLine", "V", &["Z"]),
                &[2, 4],
            );
        }
        if kind == "Memo" {
            asm.const_(4, 0);
            asm.invoke(
                "virtual",
                method("Landroid/widget/TextView;", "setSingleLine", "V", &["Z"]),
                &[2, 4],
            );
            asm.const_(4, 0x800033);
            asm.invoke(
                "virtual",
                method("Landroid/widget/TextView;", "setGravity", "V", &["I"]),
                &[2, 4],
            );
        }
        if kind == "Number" {
            asm.const_(4, 0x1002);
            asm.invoke(
                "virtual",
                method("Landroid/widget/TextView;", "setInputType", "V", &["I"]),
                &[2, 4],
            );
        }
        if kind == "Password" {
            asm.const_(4, 0x81);
            asm.invoke(
                "virtual",
                method("Landroid/widget/TextView;", "setInputType", "V", &["I"]),
                &[2, 4],
            );
        }
        if kind == "Email" {
            asm.const_(4, 0x21);
            asm.invoke(
                "virtual",
                method("Landroid/widget/TextView;", "setInputType", "V", &["I"]),
                &[2, 4],
            );
        }
        if kind == "Phone" {
            asm.const_(4, 0x3);
            asm.invoke(
                "virtual",
                method("Landroid/widget/TextView;", "setInputType", "V", &["I"]),
                &[2, 4],
            );
        }
        if kind == "Rating" {
            asm.const_(4, 1);
            asm.unop(0x82, 4, 4, "int-to-float");
            asm.invoke(
                "virtual",
                method("Landroid/widget/RatingBar;", "setStepSize", "V", &["F"]),
                &[2, 4],
            );
        }
        let props = comp.get("props").and_then(|v| v.as_object());
        // NumberPicker rejects a min above the default max of 0, so the ceiling is written first.
        let picker = [
            "MaxValue", "MinValue", "Value", "Wrap", "Enabled", "Visible",
        ];
        let order: &[&str] = if kind == "Picker" {
            &picker
        } else {
            spec.props
        };
        if let Some(map) = props {
            for prop in order {
                if kind == "Date" && matches!(*prop, "Year" | "Month" | "Day") {
                    continue;
                }
                if kind == "Web" && matches!(*prop, "Html" | "Url") {
                    continue;
                }
                if kind == "List" && matches!(*prop, "Items" | "Details") {
                    continue;
                }
                if kind == "Spinner" && matches!(*prop, "Items" | "Selection") {
                    continue;
                }
                if kind == "Panel" && *prop == "Direction" {
                    continue;
                }
                if let Some(value) = map.get(*prop) {
                    emit_const_prop(asm, 2, prop, value)?;
                }
            }
        }
        if kind == "Button" {
            let side = comp_int(comp, "width")?;
            if side == comp_int(comp, "height")? && side >= 28 {
                emit_round_button(asm, props)?;
            }
        }
        if kind == "Date" {
            // Spinner date, so the editor can sit on a note without a calendar.
            asm.const_(4, 0);
            asm.invoke(
                "virtual",
                method("Landroid/widget/DatePicker;", "setCalendarViewShown", "V", &["Z"]),
                &[2, 4],
            );
            asm.const_(4, 1);
            asm.invoke(
                "virtual",
                method("Landroid/widget/DatePicker;", "setSpinnersShown", "V", &["Z"]),
                &[2, 4],
            );
        }
        if kind == "Date" && event_of(comp, "OnChange").is_none() {
            emit_update_date(asm, props)?;
        }
        if kind == "Web" {
            emit_web_load(asm, props);
        }
        if kind == "List" {
            let text = props
                .and_then(|map| map.get("Items"))
                .map(py_str)
                .unwrap_or_default();
            asm.const_string(3, &text);
            asm.invoke(
                "static",
                method(
                    &self.rt_desc,
                    "setLines",
                    "V",
                    &[
                        "Landroid/widget/ListView;",
                        "Landroid/content/Context;",
                        "Ljava/lang/String;",
                    ],
                ),
                &[2, this, 3],
            );
            let details = props
                .and_then(|map| map.get("Details"))
                .map(py_str)
                .unwrap_or_default();
            if !details.is_empty() {
                asm.const_string(3, &details);
                asm.invoke(
                    "static",
                    method(
                        &self.rt_desc,
                        "setDetails",
                        "V",
                        &[
                            "Landroid/widget/ListView;",
                            "Landroid/content/Context;",
                            "Ljava/lang/String;",
                        ],
                    ),
                    &[2, this, 3],
                );
            }

            // The form sits in a ScrollView. Without this, that parent takes the
            // vertical drag and the list never moves.
            let scroll = class_desc(&self.package, "DroideListScroll");
            asm.new_instance(3, &scroll);
            asm.invoke("direct", method(&scroll, "<init>", "V", &[]), &[3]);
            asm.invoke(
                "virtual",
                method(
                    "Landroid/view/View;",
                    "setOnTouchListener",
                    "V",
                    &["Landroid/view/View$OnTouchListener;"],
                ),
                &[2, 3],
            );
        }
        if kind == "Spinner" {
            let text = props
                .and_then(|map| map.get("Items"))
                .map(py_str)
                .unwrap_or_default();
            asm.const_string(3, &text);
            asm.invoke(
                "static",
                method(
                    &self.rt_desc,
                    "setSpinner",
                    "V",
                    &[
                        "Landroid/widget/Spinner;",
                        "Landroid/content/Context;",
                        "Ljava/lang/String;",
                    ],
                ),
                &[2, this, 3],
            );
            let selection = match props.and_then(|map| map.get("Selection")) {
                Some(value) => json_int(value)?,
                None => 0,
            };
            asm.const_(4, selection);
            asm.invoke(
                "virtual",
                method("Landroid/widget/AdapterView;", "setSelection", "V", &["I"]),
                &[2, 4],
            );
        }
        if kind == "Panel" {
            // LinearLayout.VERTICAL is 1. HORIZONTAL is 0.
            let orient = if direction_horizontal(comp) { 0 } else { 1 };
            asm.const_(4, orient);
            asm.invoke(
                "virtual",
                method("Landroid/widget/LinearLayout;", "setOrientation", "V", &["I"]),
                &[2, 4],
            );
        }
        let parented = !comp_parent(comp).is_empty();
        let params = if parented {
            "Landroid/widget/LinearLayout$LayoutParams;"
        } else {
            "Landroid/widget/FrameLayout$LayoutParams;"
        };
        let form_h = int_or(&form.form, "height", 640)?;
        let top = comp_int(comp, "top")?;
        let height = comp_int(comp, "height")?;
        let dock = comp
            .get("props")
            .and_then(|props| props.get("Dock"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        // Fill keeps the design gap under the widget and grows with the window.
        // Bottom keeps the design gap under the widget and sits on the window bottom.
        let gap = (form_h - top - height).max(0);
        let form_w = int_or(&form.form, "width", 360)?;
        let comp_w = comp_int(comp, "width")?;
        let comp_l = comp_int(comp, "left")?;
        if !parented && comp_l.saturating_add(comp_w) >= form_w {
            asm.const_(4, -1);
        } else {
            self.emit_dp(asm, this, 4, comp_w);
        }
        if !parented && dock.eq_ignore_ascii_case("Fill") {
            asm.const_(5, -1);
        } else {
            self.emit_dp(asm, this, 5, height);
        }
        asm.new_instance(3, params);
        asm.invoke(
            "direct",
            method(params, "<init>", "V", &["I", "I"]),
            &[3, 4, 5],
        );
        self.emit_dp(asm, this, 4, comp_int(comp, "left")?);
        if !parented && dock.eq_ignore_ascii_case("Bottom") {
            asm.const_(5, 80);
            asm.iput(
                5,
                3,
                field("Landroid/widget/FrameLayout$LayoutParams;", "gravity", "I"),
            );
            self.emit_dp(asm, this, 5, gap);
            asm.iput(
                5,
                3,
                field(
                    "Landroid/view/ViewGroup$MarginLayoutParams;",
                    "bottomMargin",
                    "I",
                ),
            );
        } else if !parented && dock.eq_ignore_ascii_case("Fill") {
            self.emit_dp(asm, this, 5, top);
            asm.iput(
                5,
                3,
                field(
                    "Landroid/view/ViewGroup$MarginLayoutParams;",
                    "topMargin",
                    "I",
                ),
            );
            self.emit_dp(asm, this, 5, gap);
            asm.iput(
                5,
                3,
                field(
                    "Landroid/view/ViewGroup$MarginLayoutParams;",
                    "bottomMargin",
                    "I",
                ),
            );
        } else {
            self.emit_dp(asm, this, 5, top);
            asm.iput(
                5,
                3,
                field(
                    "Landroid/view/ViewGroup$MarginLayoutParams;",
                    "topMargin",
                    "I",
                ),
            );
        }
        asm.iput(
            4,
            3,
            field(
                "Landroid/view/ViewGroup$MarginLayoutParams;",
                "leftMargin",
                "I",
            ),
        );
        asm.invoke(
            "virtual",
            method(
                "Landroid/view/View;",
                "setLayoutParams",
                "V",
                &["Landroid/view/ViewGroup$LayoutParams;"],
            ),
            &[2, 3],
        );
        if kind == "List" {
            // After the margins exist. The bar then sits on the window's right edge.
            self.emit_dp(asm, this, 4, comp_int(comp, "width")?);
            asm.invoke(
                "static",
                method(
                    &self.rt_desc,
                    "hugScroll",
                    "V",
                    &["Landroid/view/View;", "I"],
                ),
                &[2, 4],
            );
        }
        if event_of(comp, "OnClick").is_some() {
            let ldesc = class_desc(&self.package, &format!("{}$Click${}", form.name, comp_name));
            asm.new_instance(3, &ldesc);
            asm.invoke("direct", method(&ldesc, "<init>", "V", &[desc]), &[3, this]);
            asm.invoke(
                "virtual",
                method(
                    "Landroid/view/View;",
                    "setOnClickListener",
                    "V",
                    &["Landroid/view/View$OnClickListener;"],
                ),
                &[2, 3],
            );
        }
        if event_of(comp, "OnItemClick").is_some() {
            let ldesc = class_desc(&self.package, &format!("{}$Item${}", form.name, comp_name));
            asm.new_instance(3, &ldesc);
            asm.invoke("direct", method(&ldesc, "<init>", "V", &[desc]), &[3, this]);
            asm.invoke(
                "virtual",
                method(
                    "Landroid/widget/AdapterView;",
                    "setOnItemClickListener",
                    "V",
                    &["Landroid/widget/AdapterView$OnItemClickListener;"],
                ),
                &[2, 3],
            );
        }
        if event_of(comp, "OnLongClick").is_some() {
            let ldesc = class_desc(&self.package, &format!("{}$Long${}", form.name, comp_name));
            asm.new_instance(3, &ldesc);
            asm.invoke("direct", method(&ldesc, "<init>", "V", &[desc]), &[3, this]);
            asm.invoke(
                "virtual",
                method(
                    "Landroid/view/View;",
                    "setOnLongClickListener",
                    "V",
                    &["Landroid/view/View$OnLongClickListener;"],
                ),
                &[2, 3],
            );
        }
        if event_of(comp, "OnItemLongClick").is_some() {
            let ldesc = class_desc(&self.package, &format!("{}$ItemLong${}", form.name, comp_name));
            asm.new_instance(3, &ldesc);
            asm.invoke("direct", method(&ldesc, "<init>", "V", &[desc]), &[3, this]);
            asm.invoke(
                "virtual",
                method(
                    "Landroid/widget/AdapterView;",
                    "setOnItemLongClickListener",
                    "V",
                    &["Landroid/widget/AdapterView$OnItemLongClickListener;"],
                ),
                &[2, 3],
            );
        }
        if event_of(comp, "OnItemSelected").is_some() {
            let ldesc = class_desc(&self.package, &format!("{}$Select${}", form.name, comp_name));
            asm.new_instance(3, &ldesc);
            asm.invoke("direct", method(&ldesc, "<init>", "V", &[desc]), &[3, this]);
            asm.invoke(
                "virtual",
                method(
                    "Landroid/widget/AdapterView;",
                    "setOnItemSelectedListener",
                    "V",
                    &["Landroid/widget/AdapterView$OnItemSelectedListener;"],
                ),
                &[2, 3],
            );
        }
        if event_of(comp, "OnChange").is_some() {
            if kind == "Date" {
                self.wire_date(asm, form, desc, this, comp_name, props)?;
            } else {
                self.wire_change(asm, form, desc, this, kind, comp_name)?;
            }
        }
        if event_of(comp, "OnEnter").is_some() {
            self.wire_enter(asm, form, desc, this, kind, comp_name)?;
        }
        if !parented {
            asm.invoke(
                "virtual",
                method(
                    "Landroid/view/ViewGroup;",
                    "addView",
                    "V",
                    &["Landroid/view/View;"],
                ),
                &[1, 2],
            );
        }
        asm.iput_obj(2, this, field(desc, comp_name, wdesc));
        Ok(())
    }

    /// Children are attached after every widget field exists, so a child may
    /// be listed before its panel. left and top are LinearLayout margins.
    fn place_children(
        &self,
        asm: &mut Asm,
        desc: &str,
        this: u16,
        comps: &[Value],
    ) -> Result<(), CompileError> {
        for comp in comps {
            let parent_name = comp_parent(comp);
            if parent_name.is_empty() || comp.get("type").and_then(|v| v.as_str()) == Some("Timer") {
                continue;
            }
            let child_name = comp_str(comp, "name")?;
            let child_desc = spec_exact(comp_str(comp, "type")?)?.desc;
            let parent = comps.iter().find(|other| {
                other
                    .get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(parent_name))
            });
            let Some(parent) = parent else {
                return Err(CompileError::bare(format!(
                    "{child_name} parent {parent_name} is unknown"
                )));
            };
            let parent_field = comp_str(parent, "name")?;
            let parent_desc = spec_exact(comp_str(parent, "type")?)?.desc;
            asm.iget_obj(3, this, field(desc, parent_field, parent_desc));
            asm.iget_obj(2, this, field(desc, child_name, child_desc));
            asm.invoke(
                "virtual",
                method(
                    "Landroid/view/ViewGroup;",
                    "addView",
                    "V",
                    &["Landroid/view/View;"],
                ),
                &[3, 2],
            );
        }
        Ok(())
    }

    fn emit_dp(&self, asm: &mut Asm, this: u16, dest: u16, px: i64) {
        asm.const_(dest, px);
        asm.invoke(
            "static",
            method(
                &self.rt_desc,
                "dp",
                "I",
                &["Landroid/content/Context;", "I"],
            ),
            &[this, dest],
        );
        asm.move_result(dest, false);
    }

    fn listener_shell(
        &self,
        form_name: &str,
        suffix: &str,
        widget: &str,
        form_desc: &str,
        iface: &str,
    ) -> (String, ClassModel) {
        let lname = format!("{form_name}${suffix}${widget}");
        let desc = class_desc(&self.package, &lname);
        let source = format!("{form_name}.droid");
        let mut cls = ClassModel::new(&desc, "Ljava/lang/Object;", ACC_PUBLIC, &source);
        cls.interfaces().push(iface.to_string());
        cls.field("this$0", form_desc, ACC_PUBLIC);
        let mut init = Asm::new();
        init.invoke(
            "direct",
            method("Ljava/lang/Object;", "<init>", "V", &[]),
            &[0],
        );
        init.iput_obj(1, 0, field(&desc, "this$0", form_desc));
        init.ret(0, "void");
        cls.method(MethodModel::new(
            "<init>",
            "V",
            vec![form_desc.to_string()],
            ACC_PUBLIC | ACC_CONSTRUCTOR,
            Some(init),
            2,
            2,
            1,
        ));
        (desc, cls)
    }

    fn wire_enter(
        &self,
        asm: &mut Asm,
        form: &FormChecked,
        form_desc: &str,
        this: u16,
        kind: &str,
        comp_name: &str,
    ) -> Result<(), CompileError> {
        let (owner, name, listener) = change_hook(kind)
            .ok_or_else(|| CompileError::bare(format!("{kind} has no OnEnter event")))?;
        let ldesc = class_desc(&self.package, &format!("{}$Enter${}", form.name, comp_name));
        asm.new_instance(3, &ldesc);
        asm.invoke("direct", method(&ldesc, "<init>", "V", &[form_desc]), &[3, this]);
        asm.invoke(
            "virtual",
            method(owner, name, "V", &[listener]),
            &[2, 3],
        );
        Ok(())
    }

    fn wire_change(
        &self,
        asm: &mut Asm,
        form: &FormChecked,
        form_desc: &str,
        this: u16,
        kind: &str,
        comp_name: &str,
    ) -> Result<(), CompileError> {
        let (owner, name, listener) = change_hook(kind)
            .ok_or_else(|| CompileError::bare(format!("{kind} has no OnChange event")))?;
        let ldesc = class_desc(&self.package, &format!("{}$Change${}", form.name, comp_name));
        asm.new_instance(3, &ldesc);
        asm.invoke("direct", method(&ldesc, "<init>", "V", &[form_desc]), &[3, this]);
        asm.invoke(
            "virtual",
            method(owner, name, "V", &[listener]),
            &[2, 3],
        );
        Ok(())
    }

    fn wire_date(
        &self,
        asm: &mut Asm,
        form: &FormChecked,
        form_desc: &str,
        this: u16,
        comp_name: &str,
        props: Option<&Map<String, Value>>,
    ) -> Result<(), CompileError> {
        let ldesc = class_desc(&self.package, &format!("{}$Change${}", form.name, comp_name));
        asm.new_instance(7, &ldesc);
        asm.invoke("direct", method(&ldesc, "<init>", "V", &[form_desc]), &[7, this]);
        let defaults = default_props("Date");
        let year = prop_or(props, &defaults, "Year")?;
        let month = prop_or(props, &defaults, "Month")?;
        let day = prop_or(props, &defaults, "Day")?;
        asm.const_(3, year);
        asm.const_(4, month - 1);
        asm.const_(5, day);
        asm.invoke(
            "virtual",
            method(
                "Landroid/widget/DatePicker;",
                "init",
                "V",
                &[
                    "I",
                    "I",
                    "I",
                    "Landroid/widget/DatePicker$OnDateChangedListener;",
                ],
            ),
            &[2, 3, 4, 5, 7],
        );
        Ok(())
    }

    fn long_listener(
        &self,
        form: &FormChecked,
        form_desc: &str,
        widget: &WidgetRef,
        handler: &str,
    ) -> Result<ClassModel, CompileError> {
        let (desc, mut cls) = self.listener_shell(
            &form.name,
            "Long",
            &widget.name,
            form_desc,
            "Landroid/view/View$OnLongClickListener;",
        );
        let rec = form
            .routines
            .get(&handler.to_lowercase())
            .ok_or_else(|| CompileError::bare(format!("{handler} is not declared")))?;
        let mut click = Asm::new();
        click.iget_obj(0, 1, field(&desc, "this$0", form_desc));
        let key = handler_key(form_desc, rec)?;
        let outs = if rec.params.is_empty() {
            click.invoke("virtual", key, &[0]);
            1
        } else {
            click.invoke("virtual", key, &[0, 2]);
            2
        };
        click.const_(0, 1);
        click.ret(0, "int");
        cls.method(MethodModel::new(
            "onLongClick",
            "Z",
            vec!["Landroid/view/View;".into()],
            ACC_PUBLIC,
            Some(click),
            3,
            2,
            outs,
        ));
        Ok(cls)
    }

    fn item_long_listener(
        &self,
        form: &FormChecked,
        form_desc: &str,
        widget: &WidgetRef,
        handler: &str,
    ) -> Result<ClassModel, CompileError> {
        let (desc, mut cls) = self.listener_shell(
            &form.name,
            "ItemLong",
            &widget.name,
            form_desc,
            "Landroid/widget/AdapterView$OnItemLongClickListener;",
        );
        let rec = form
            .routines
            .get(&handler.to_lowercase())
            .ok_or_else(|| CompileError::bare(format!("{handler} is not declared")))?;
        let mut click = Asm::new();
        click.iget_obj(0, 2, field(&desc, "this$0", form_desc));
        let key = handler_key(form_desc, rec)?;
        let outs = match rec.params.first().map(String::as_str) {
            None => {
                click.invoke("virtual", key, &[0]);
                1
            }
            Some("Integer") => {
                click.invoke("virtual", key, &[0, 5]);
                2
            }
            Some("View") => {
                click.invoke("virtual", key, &[0, 4]);
                2
            }
            Some("String") => {
                click.invoke(
                    "virtual",
                    method(
                        "Landroid/widget/AdapterView;",
                        "getItemAtPosition",
                        "Ljava/lang/Object;",
                        &["I"],
                    ),
                    &[3, 5],
                );
                click.move_result(1, true);
                click.invoke(
                    "virtual",
                    method("Ljava/lang/Object;", "toString", "Ljava/lang/String;", &[]),
                    &[1],
                );
                click.move_result(1, true);
                click.invoke("virtual", key, &[0, 1]);
                2
            }
            Some(other) => {
                return Err(CompileError::bare(format!(
                    "OnItemLongClick parameter {other} is not supported"
                )));
            }
        };
        click.const_(0, 1);
        click.ret(0, "int");
        cls.method(MethodModel::new(
            "onItemLongClick",
            "Z",
            vec![
                "Landroid/widget/AdapterView;".into(),
                "Landroid/view/View;".into(),
                "I".into(),
                "J".into(),
            ],
            ACC_PUBLIC,
            Some(click),
            8,
            6,
            outs,
        ));
        Ok(cls)
    }

    fn enter_listener(
        &self,
        form: &FormChecked,
        form_desc: &str,
        widget: &WidgetRef,
        handler: &str,
    ) -> Result<ClassModel, CompileError> {
        let kind = comp_str(&widget.comp, "type")?;
        if !matches!(kind, "Edit" | "Memo" | "Number" | "Password" | "Email" | "Phone") {
            return Err(CompileError::bare(format!("{kind} has no OnEnter event")));
        }
        let rec = form
            .routines
            .get(&handler.to_lowercase())
            .ok_or_else(|| CompileError::bare(format!("{handler} is not declared")))?;
        let (desc, mut cls) = self.listener_shell(
            &form.name,
            "Enter",
            &widget.name,
            form_desc,
            "Landroid/text/TextWatcher;",
        );
        emit_enter_watcher(&mut cls, &desc, form_desc, rec)?;
        Ok(cls)
    }

    fn change_listener(
        &self,
        form: &FormChecked,
        form_desc: &str,
        widget: &WidgetRef,
        handler: &str,
    ) -> Result<ClassModel, CompileError> {
        let kind = comp_str(&widget.comp, "type")?;
        let rec = form
            .routines
            .get(&handler.to_lowercase())
            .ok_or_else(|| CompileError::bare(format!("{handler} is not declared")))?;
        let (desc, mut cls) = self.listener_shell(
            &form.name,
            "Change",
            &widget.name,
            form_desc,
            change_iface(kind),
        );
        match kind {
            "Edit" | "Memo" | "Number" | "Password" | "Email" | "Phone" => {
                emit_text_watcher(&mut cls, &desc, form_desc, rec)?;
            }
            "Check" | "Radio" | "Switch" | "Toggle" => {
                emit_checked(&mut cls, &desc, form_desc, rec)?;
            }
            "Seek" => emit_seek(&mut cls, &desc, form_desc, rec)?,
            "Rating" => emit_rating(&mut cls, &desc, form_desc, rec)?,
            "Picker" => emit_picker(&mut cls, &desc, form_desc, rec)?,
            "Time" => emit_time(&mut cls, &desc, form_desc, rec)?,
            "Date" => emit_date_changed(&mut cls, &desc, form_desc, rec)?,
            other => {
                return Err(CompileError::bare(format!("{other} has no OnChange event")));
            }
        }
        Ok(cls)
    }

    fn listener(
        &self,
        form: &FormChecked,
        form_desc: &str,
        widget: &WidgetRef,
        handler: &str,
    ) -> Result<ClassModel, CompileError> {
        let lname = format!("{}$Click${}", form.name, widget.name);
        let desc = class_desc(&self.package, &lname);
        let source = format!("{}.droid", form.name);
        let mut cls = ClassModel::new(&desc, "Ljava/lang/Object;", ACC_PUBLIC, &source);
        cls.interfaces()
            .push("Landroid/view/View$OnClickListener;".to_string());
        cls.field("this$0", form_desc, ACC_PUBLIC);
        let mut init = Asm::new();
        init.invoke(
            "direct",
            method("Ljava/lang/Object;", "<init>", "V", &[]),
            &[0],
        );
        init.iput_obj(1, 0, field(&desc, "this$0", form_desc));
        init.ret(0, "void");
        cls.method(MethodModel::new(
            "<init>",
            "V",
            vec![form_desc.to_string()],
            ACC_PUBLIC | ACC_CONSTRUCTOR,
            Some(init),
            2,
            2,
            1,
        ));
        let rec = form
            .routines
            .get(&handler.to_lowercase())
            .ok_or_else(|| CompileError::bare(format!("{handler} is not declared")))?;
        let mut click = Asm::new();
        // v1 = this, v2 = view, v0 = outer form.
        click.iget_obj(0, 1, field(&desc, "this$0", form_desc));
        let key = handler_key(form_desc, rec)?;
        let outs = if rec.params.is_empty() {
            click.invoke("virtual", key, &[0]);
            1
        } else {
            click.invoke("virtual", key, &[0, 2]);
            2
        };
        click.ret(0, "void");
        cls.method(MethodModel::new(
            "onClick",
            "V",
            vec!["Landroid/view/View;".into()],
            ACC_PUBLIC,
            Some(click),
            3,
            2,
            outs,
        ));
        Ok(cls)
    }

    fn ask_listener(
        &self,
        form: &FormChecked,
        form_desc: &str,
        handler: &str,
    ) -> Result<ClassModel, CompileError> {
        let lname = format!("{}$Ask${handler}", form.name);
        let desc = class_desc(&self.package, &lname);
        let source = format!("{}.droid", form.name);
        let mut cls = ClassModel::new(&desc, "Ljava/lang/Object;", ACC_PUBLIC, &source);
        cls.interfaces()
            .push("Landroid/content/DialogInterface$OnClickListener;".to_string());
        cls.field("this$0", form_desc, ACC_PUBLIC);
        let mut init = Asm::new();
        init.invoke(
            "direct",
            method("Ljava/lang/Object;", "<init>", "V", &[]),
            &[0],
        );
        init.iput_obj(1, 0, field(&desc, "this$0", form_desc));
        init.ret(0, "void");
        cls.method(MethodModel::new(
            "<init>",
            "V",
            vec![form_desc.to_string()],
            ACC_PUBLIC | ACC_CONSTRUCTOR,
            Some(init),
            2,
            2,
            1,
        ));
        let rec = form.routines.get(&handler.to_lowercase()).ok_or_else(|| {
            CompileError::bare(format!("{handler} is not declared"))
        })?;
        if rec.ret.is_some() {
            return Err(CompileError::bare(format!(
                "{handler} must be a procedure"
            )));
        }
        if !rec.params.is_empty() {
            return Err(CompileError::bare(format!(
                "{handler} takes no parameters"
            )));
        }
        let mut click = Asm::new();
        // v1 listener, v2 dialog, v3 which. v0 is the form. registers 4, ins 3.
        click.iget_obj(0, 1, field(&desc, "this$0", form_desc));
        let key = handler_key(form_desc, rec)?;
        click.invoke("virtual", key, &[0]);
        click.ret(0, "void");
        cls.method(MethodModel::new(
            "onClick",
            "V",
            vec!["Landroid/content/DialogInterface;".into(), "I".into()],
            ACC_PUBLIC,
            Some(click),
            4,
            3,
            1,
        ));
        Ok(cls)
    }

    fn item_listener(
        &self,
        form: &FormChecked,
        form_desc: &str,
        widget: &WidgetRef,
        handler: &str,
    ) -> Result<ClassModel, CompileError> {
        let lname = format!("{}$Item${}", form.name, widget.name);
        let desc = class_desc(&self.package, &lname);
        let source = format!("{}.droid", form.name);
        let mut cls = ClassModel::new(&desc, "Ljava/lang/Object;", ACC_PUBLIC, &source);
        cls.interfaces()
            .push("Landroid/widget/AdapterView$OnItemClickListener;".to_string());
        cls.field("this$0", form_desc, ACC_PUBLIC);
        let mut init = Asm::new();
        init.invoke(
            "direct",
            method("Ljava/lang/Object;", "<init>", "V", &[]),
            &[0],
        );
        init.iput_obj(1, 0, field(&desc, "this$0", form_desc));
        init.ret(0, "void");
        cls.method(MethodModel::new(
            "<init>",
            "V",
            vec![form_desc.to_string()],
            ACC_PUBLIC | ACC_CONSTRUCTOR,
            Some(init),
            2,
            2,
            1,
        ));
        let rec = form
            .routines
            .get(&handler.to_lowercase())
            .ok_or_else(|| CompileError::bare(format!("{handler} is not declared")))?;
        let mut click = Asm::new();
        // registers 8, ins 6. v2 this, v3 parent, v4 row, v5 position, v6-v7 the long id.
        // v0 is the form. v1 is the row text.
        click.iget_obj(0, 2, field(&desc, "this$0", form_desc));
        let key = handler_key(form_desc, rec)?;
        let outs = match rec.params.first().map(String::as_str) {
            None => {
                click.invoke("virtual", key, &[0]);
                1
            }
            Some("Integer") => {
                click.invoke("virtual", key, &[0, 5]);
                2
            }
            Some("View") => {
                click.invoke("virtual", key, &[0, 4]);
                2
            }
            Some("String") => {
                click.invoke(
                    "virtual",
                    method(
                        "Landroid/widget/AdapterView;",
                        "getItemAtPosition",
                        "Ljava/lang/Object;",
                        &["I"],
                    ),
                    &[3, 5],
                );
                click.move_result(1, true);
                click.invoke(
                    "virtual",
                    method("Ljava/lang/Object;", "toString", "Ljava/lang/String;", &[]),
                    &[1],
                );
                click.move_result(1, true);
                click.invoke("virtual", key, &[0, 1]);
                2
            }
            Some(other) => {
                return Err(CompileError::bare(format!(
                    "OnItemClick parameter {other} is not supported"
                )));
            }
        };
        click.ret(0, "void");
        cls.method(MethodModel::new(
            "onItemClick",
            "V",
            vec![
                "Landroid/widget/AdapterView;".into(),
                "Landroid/view/View;".into(),
                "I".into(),
                "J".into(),
            ],
            ACC_PUBLIC,
            Some(click),
            8,
            6,
            outs,
        ));
        Ok(cls)
    }

    fn select_listener(
        &self,
        form: &FormChecked,
        form_desc: &str,
        widget: &WidgetRef,
        handler: &str,
    ) -> Result<ClassModel, CompileError> {
        let lname = format!("{}$Select${}", form.name, widget.name);
        let desc = class_desc(&self.package, &lname);
        let source = format!("{}.droid", form.name);
        let mut cls = ClassModel::new(&desc, "Ljava/lang/Object;", ACC_PUBLIC, &source);
        cls.interfaces()
            .push("Landroid/widget/AdapterView$OnItemSelectedListener;".to_string());
        cls.field("this$0", form_desc, ACC_PUBLIC);
        let mut init = Asm::new();
        init.invoke(
            "direct",
            method("Ljava/lang/Object;", "<init>", "V", &[]),
            &[0],
        );
        init.iput_obj(1, 0, field(&desc, "this$0", form_desc));
        init.ret(0, "void");
        cls.method(MethodModel::new(
            "<init>",
            "V",
            vec![form_desc.to_string()],
            ACC_PUBLIC | ACC_CONSTRUCTOR,
            Some(init),
            2,
            2,
            1,
        ));
        let rec = form
            .routines
            .get(&handler.to_lowercase())
            .ok_or_else(|| CompileError::bare(format!("{handler} is not declared")))?;
        let mut click = Asm::new();
        // registers 8, ins 6. v2 this, v3 parent, v4 row, v5 position, v6-v7 the long id.
        click.iget_obj(0, 2, field(&desc, "this$0", form_desc));
        let key = handler_key(form_desc, rec)?;
        let outs = match rec.params.first().map(String::as_str) {
            None => {
                click.invoke("virtual", key, &[0]);
                1
            }
            Some("Integer") => {
                click.invoke("virtual", key, &[0, 5]);
                2
            }
            Some("View") => {
                click.invoke("virtual", key, &[0, 4]);
                2
            }
            Some("String") => {
                click.invoke(
                    "virtual",
                    method(
                        "Landroid/widget/AdapterView;",
                        "getItemAtPosition",
                        "Ljava/lang/Object;",
                        &["I"],
                    ),
                    &[3, 5],
                );
                click.move_result(1, true);
                click.invoke(
                    "virtual",
                    method("Ljava/lang/Object;", "toString", "Ljava/lang/String;", &[]),
                    &[1],
                );
                click.move_result(1, true);
                click.invoke("virtual", key, &[0, 1]);
                2
            }
            Some(other) => {
                return Err(CompileError::bare(format!(
                    "OnItemSelected parameter {other} is not supported"
                )));
            }
        };
        click.ret(0, "void");
        cls.method(MethodModel::new(
            "onItemSelected",
            "V",
            vec![
                "Landroid/widget/AdapterView;".into(),
                "Landroid/view/View;".into(),
                "I".into(),
                "J".into(),
            ],
            ACC_PUBLIC,
            Some(click),
            8,
            6,
            outs,
        ));
        empty_method(
            &mut cls,
            "onNothingSelected",
            &["Landroid/widget/AdapterView;"],
            2,
            2,
        );
        Ok(cls)
    }
}

impl<'a> Gen<'a> {
    fn new(
        asm: &'a mut Asm,
        frame: &'a Frame,
        lab: &'a mut u32,
        owner: &str,
        package: &str,
        native_desc: &str,
        result_codes: &[ResultUse],
    ) -> Self {
        *lab += 1;
        let ret_label = format!("L{lab}");
        Self {
            asm,
            frame,
            lab,
            owner: owner.to_string(),
            package: package.to_string(),
            native_desc: native_desc.to_string(),
            result_codes: result_codes.to_vec(),
            sp: 0,
            outs: 0,
            ret_label,
        }
    }

    fn fresh(&mut self) -> String {
        *self.lab += 1;
        format!("L{}", self.lab)
    }

    /// A false result from the runtime means nothing was started, so the handler
    /// runs now with an empty path. A start delivers the path from onActivityResult.
    fn emit_activity_result(
        &mut self,
        name: &str,
        form_name: Option<&str>,
        start: u16,
    ) -> Result<Option<Val>, CompileError> {
        let photo = name == "TakePhoto";
        let handler = form_name.filter(|item| !item.is_empty()).ok_or_else(|| {
            CompileError::bare(format!("{name} needs a procedure name"))
        })?;
        let this = self.frame.this.ok_or_else(|| {
            CompileError::bare(format!("{name} outside a form"))
        })?;
        let code = self
            .result_codes
            .iter()
            .find(|item| item.photo == photo && item.handler.eq_ignore_ascii_case(handler))
            .map(|item| item.code)
            .ok_or_else(|| CompileError::bare(format!("{name} needs a procedure name")))?;
        self.release_to(start);
        let code_reg = self.alloc()?;
        self.asm.const_(code_reg, code);
        let owner = class_desc(&self.package, "DroideRt");
        let runtime = if photo { "takePhoto" } else { "openDocument" };
        self.invoke(
            "static",
            method(&owner, runtime, "Z", &["Landroid/app/Activity;", "I"]),
            &[this, code_reg],
        );
        self.asm.move_result(code_reg, false);
        let started = self.fresh();
        self.asm.if_zero("nez", code_reg, &started);
        let path = self.alloc()?;
        self.asm.const_string(path, "");
        self.invoke(
            "virtual",
            method(&self.owner, handler, "V", &["Ljava/lang/String;"]),
            &[this, path],
        );
        self.asm.label(&started);
        self.release_to(start);
        Ok(None)
    }

    fn alloc(&mut self) -> Result<u16, CompileError> {
        if self.sp >= SCRATCH {
            return Err(CompileError::bare("expression is too complex"));
        }
        let reg = self.sp;
        self.sp += 1;
        Ok(reg)
    }

    fn release_to(&mut self, n: u16) {
        while self.sp > n {
            self.sp -= 1;
            self.asm.const_(self.sp, 0);
        }
    }

    fn zero_except(&mut self, keep: &[u16]) {
        for reg in 0..self.sp {
            if !keep.contains(&reg) {
                self.asm.const_(reg, 0);
            }
        }
    }

    fn invoke(&mut self, kind: &str, key: MethodRef, regs: &[u16]) {
        let n = regs.len() as u16;
        self.outs = self.outs.max(if n == 0 { 1 } else { n });
        self.asm.invoke(kind, key, regs);
    }

    fn prologue(&mut self) {
        for reg in 0..SCRATCH {
            self.asm.const_(reg, 0);
        }
        let inits: Vec<(u16, bool)> = self
            .frame
            .order
            .iter()
            .filter_map(|name| {
                if self.frame.param_names.contains(name) {
                    return None;
                }
                self.frame
                    .slots
                    .get(name)
                    .map(|(reg, ty)| (*reg, ty == "String"))
            })
            .collect();
        for (reg, string) in inits {
            if string {
                self.asm.const_string(reg, "");
            } else {
                self.asm.const_(reg, 0);
            }
        }
    }

    fn finish(&mut self, ret: Option<&str>) {
        let ret_label = self.ret_label.clone();
        self.asm.label(&ret_label);
        let Some(ret) = ret else {
            self.asm.ret(0, "void");
            return;
        };
        let slot = self
            .frame
            .slots
            .get("result")
            .map(|(reg, ty)| (*reg, is_obj(ty)));
        if let Some((reg, obj)) = slot {
            self.asm.ret(reg, if obj { "obj" } else { "int" });
            return;
        }
        self.asm.ret(0, if is_obj(ret) { "obj" } else { "int" });
    }

    fn emit_body(&mut self, stmts: &[Stmt]) -> Result<(), CompileError> {
        self.emit_stmts(stmts)?;
        self.release_to(0);
        let ret_label = self.ret_label.clone();
        self.asm.goto(&ret_label);
        Ok(())
    }

    fn emit_stmts(&mut self, stmts: &[Stmt]) -> Result<(), CompileError> {
        for stmt in stmts {
            self.emit_stmt(stmt)?;
        }
        Ok(())
    }

    fn emit_stmt(&mut self, stmt: &Stmt) -> Result<(), CompileError> {
        match stmt {
            Stmt::Block(block) => self.emit_stmts(&block.stmts)?,
            Stmt::If(stmt) => self.emit_if(stmt)?,
            Stmt::While(stmt) => self.emit_while(stmt)?,
            Stmt::Exit(_) => {
                self.release_to(0);
                let ret_label = self.ret_label.clone();
                self.asm.goto(&ret_label);
            }
            Stmt::Assign(stmt) => {
                self.emit_assign(stmt)?;
                self.release_to(0);
            }
            Stmt::Call(stmt) => {
                self.eval_call(
                    stmt.resolved.as_ref(),
                    &stmt.args,
                    stmt.form_name.as_deref(),
                    true,
                )?;
                self.release_to(0);
            }
        }
        Ok(())
    }

    fn emit_if(&mut self, stmt: &IfStmt) -> Result<(), CompileError> {
        let (reg, _) = self.eval(&stmt.cond)?;
        let else_l = self.fresh();
        let end_l = self.fresh();
        self.zero_except(&[reg]);
        self.asm.if_zero("eqz", reg, &else_l);
        self.asm.const_(reg, 0);
        self.sp = 0;
        self.emit_stmt(&stmt.then)?;
        self.release_to(0);
        self.asm.goto(&end_l);
        self.asm.label(&else_l);
        self.asm.const_(reg, 0);
        self.sp = 0;
        self.emit_stmt(&stmt.else_)?;
        self.release_to(0);
        self.asm.label(&end_l);
        self.sp = 0;
        Ok(())
    }

    fn emit_while(&mut self, stmt: &WhileStmt) -> Result<(), CompileError> {
        let head = self.fresh();
        let end = self.fresh();
        self.asm.label(&head);
        let (reg, _) = self.eval(&stmt.cond)?;
        self.zero_except(&[reg]);
        self.asm.if_zero("eqz", reg, &end);
        self.asm.const_(reg, 0);
        self.sp = 0;
        self.emit_stmt(&stmt.body)?;
        self.release_to(0);
        self.asm.goto(&head);
        self.asm.label(&end);
        self.asm.const_(reg, 0);
        self.sp = 0;
        Ok(())
    }

    fn emit_assign(&mut self, stmt: &Assign) -> Result<(), CompileError> {
        match stmt.target.resolved.clone() {
            Some(Resolved::Prop { actual, widget }) => {
                let start = self.sp;
                let wreg = self.alloc()?;
                self.load_widget(wreg, &widget)?;
                let (vreg, vty) = self.eval(&stmt.expr)?;
                let kind = comp_str(&widget.comp, "type")?;
                self.emit_setter(wreg, &actual, vreg, kind)?;
                self.release_to(start);
                let _ = vty;
                Ok(())
            }
            Some(Resolved::SlotWrite { .. }) => {
                let (vreg, vty) = self.eval(&stmt.expr)?;
                self.store(stmt.target.name(), vreg, &vty)
            }
            _ => Err(CompileError::bare("cannot compile statement")),
        }
    }

    fn store(&mut self, name: &str, src: u16, ty: &str) -> Result<(), CompileError> {
        let (reg, slot_ty) = self.frame.get(&name.to_lowercase())?;
        let obj = is_obj(if slot_ty.is_empty() { ty } else { &slot_ty });
        self.asm.move_(reg, src, obj);
        Ok(())
    }

    fn load_widget(&mut self, dest: u16, widget: &WidgetRef) -> Result<(), CompileError> {
        let kind = comp_str(&widget.comp, "type")?;
        let spec = spec_exact(kind)?;
        let this = self
            .frame
            .this
            .ok_or_else(|| CompileError::bare("widget access outside a form"))?;
        let field_desc = component_desc(
            &self.package,
            simple_class(&self.owner),
            kind,
            &widget.name,
            spec.desc,
        );
        self.asm
            .iget_obj(dest, this, field(&self.owner, &widget.name, &field_desc));
        Ok(())
    }

    fn emit_setter(&mut self, wreg: u16, prop: &str, vreg: u16, kind: &str) -> Result<(), CompileError> {
        if kind == "Timer" {
            let tick = tick_desc(&self.package);
            let (name, desc) = if prop.eq_ignore_ascii_case("Enabled") {
                ("setEnabled", "Z")
            } else if prop.eq_ignore_ascii_case("Interval") {
                ("setInterval", "I")
            } else {
                return Err(CompileError::bare(format!("Timer has no property {prop}")));
            };
            self.invoke("virtual", method(&tick, name, "V", &[desc]), &[wreg, vreg]);
            return Ok(());
        }
        let spec = setter_ref(prop)?;
        if spec.mode == "clock" {
            let text = &["Ljava/lang/CharSequence;"];
            self.invoke(
                "virtual",
                method("Landroid/widget/TextClock;", "setFormat12Hour", "V", text),
                &[wreg, vreg],
            );
            self.invoke(
                "virtual",
                method("Landroid/widget/TextClock;", "setFormat24Hour", "V", text),
                &[wreg, vreg],
            );
            return Ok(());
        }
        if matches!(spec.mode, "date-year" | "date-month" | "date-day") {
            return self.emit_date_assign(wreg, spec.mode, vreg);
        }
        if matches!(spec.mode, "web-html" | "web-url") {
            return self.emit_web_assign(wreg, vreg, spec.mode);
        }
        if spec.mode == "orient" {
            // Horizontal is orientation 0. Anything else, including Vertical, is 1.
            let vertical = self.fresh();
            let apply = self.fresh();
            let text = self.alloc()?;
            self.asm.const_string(text, "Horizontal");
            self.invoke(
                "virtual",
                method(
                    "Ljava/lang/String;",
                    "equalsIgnoreCase",
                    "Z",
                    &["Ljava/lang/String;"],
                ),
                &[vreg, text],
            );
            self.asm.move_result(text, false);
            self.zero_except(&[wreg, text]);
            self.asm.if_zero("eqz", text, &vertical);
            self.asm.const_(text, 0);
            self.asm.goto(&apply);
            self.asm.label(&vertical);
            self.asm.const_(text, 1);
            self.asm.label(&apply);
            self.invoke(
                "virtual",
                method("Landroid/widget/LinearLayout;", "setOrientation", "V", &["I"]),
                &[wreg, text],
            );
            return Ok(());
        }
        if spec.mode == "stack" {
            self.emit_list_bottom(wreg, vreg)?;
            return Ok(());
        }
        if spec.mode == "row" {
            let which = match prop.to_lowercase().as_str() {
                "paper" => 0,
                "card" => 1,
                "ink" => 2,
                "muted" => 3,
                _ => 4,
            };
            let slot = self.alloc()?;
            self.asm.const_(slot, which);
            let rt = class_desc(&self.package, "DroideRt");
            self.invoke(
                "static",
                method(&rt, "paintRow", "V", &["Landroid/widget/ListView;", "I", "I"]),
                &[wreg, slot, vreg],
            );
            return Ok(());
        }
        if spec.mode == "dp-h" {
            let ctx = self
                .frame
                .this
                .ok_or_else(|| CompileError::bare("widget access outside a form"))?;
            let rt = class_desc(&self.package, "DroideRt");
            self.invoke(
                "static",
                method(
                    &rt,
                    "setDpHeight",
                    "V",
                    &["Landroid/view/View;", "Landroid/content/Context;", "I"],
                ),
                &[wreg, ctx, vreg],
            );
            return Ok(());
        }
        if spec.mode == "picked" {
            let rt = class_desc(&self.package, "DroideRt");
            self.invoke(
                "static",
                method(&rt, "paintPicked", "V", &["Landroid/widget/ListView;", "Ljava/lang/String;"]),
                &[wreg, vreg],
            );
            return Ok(());
        }
        if spec.mode == "recolor" {
            let rt = class_desc(&self.package, "DroideRt");
            self.invoke(
                "static",
                method(&rt, "recolor", "V", &["Landroid/view/View;", "I"]),
                &[wreg, vreg],
            );
            return Ok(());
        }
        if spec.mode == "lines" || spec.mode == "details" {
            let ctx = self
                .frame
                .this
                .ok_or_else(|| CompileError::bare("widget access outside a form"))?;
            let rt = class_desc(&self.package, "DroideRt");
            let (name, view) = if spec.mode == "details" {
                ("setDetails", "Landroid/widget/ListView;")
            } else if kind == "Spinner" {
                ("setSpinner", "Landroid/widget/Spinner;")
            } else {
                ("setLines", "Landroid/widget/ListView;")
            };
            self.invoke(
                "static",
                method(
                    &rt,
                    name,
                    "V",
                    &[view, "Landroid/content/Context;", "Ljava/lang/String;"],
                ),
                &[wreg, ctx, vreg],
            );
            return Ok(());
        }
        let key = dex_method(spec);
        if spec.mode == "boxbool" {
            // TimePicker.setIs24HourView takes java.lang.Boolean, not Z.
            self.invoke(
                "static",
                method(
                    "Ljava/lang/Boolean;",
                    "valueOf",
                    "Ljava/lang/Boolean;",
                    &["Z"],
                ),
                &[vreg],
            );
            let boxed = self.alloc()?;
            self.asm.move_result(boxed, true);
            self.invoke("virtual", key, &[wreg, boxed]);
            return Ok(());
        }
        if spec.mode == "float" {
            self.asm.unop(0x82, vreg, vreg, "int-to-float");
        } else if spec.mode == "vis" {
            let no = self.fresh();
            let end = self.fresh();
            self.zero_except(&[wreg, vreg]);
            self.asm.if_zero("eqz", vreg, &no);
            self.asm.const_(vreg, 0);
            self.asm.goto(&end);
            self.asm.label(&no);
            self.asm.const_(vreg, 8);
            self.asm.label(&end);
        }
        self.invoke("virtual", key, &[wreg, vreg]);
        Ok(())
    }

    fn emit_list_bottom(&mut self, wreg: u16, vreg: u16) -> Result<(), CompileError> {
        let off = self.fresh();
        let done = self.fresh();
        let start = self.sp;
        let mode = self.alloc()?;
        self.zero_except(&[wreg, vreg]);
        self.asm.if_zero("eqz", vreg, &off);
        self.asm.const_(mode, 2);
        self.asm.goto(&done);
        self.asm.label(&off);
        self.asm.const_(mode, 0);
        self.asm.label(&done);
        self.invoke(
            "virtual",
            method(
                "Landroid/widget/AbsListView;",
                "setStackFromBottom",
                "V",
                &["Z"],
            ),
            &[wreg, vreg],
        );
        self.invoke(
            "virtual",
            method(
                "Landroid/widget/AbsListView;",
                "setTranscriptMode",
                "V",
                &["I"],
            ),
            &[wreg, mode],
        );
        self.release_to(start);
        Ok(())
    }

    fn emit_date_assign(&mut self, wreg: u16, mode: &str, vreg: u16) -> Result<(), CompileError> {
        // updateDate replaces the whole date. A Month write is 1–12 and is stored as month − 1.
        let year = self.alloc()?;
        let month = self.alloc()?;
        let day = self.alloc()?;
        if mode != "date-year" {
            self.invoke(
                "virtual",
                method("Landroid/widget/DatePicker;", "getYear", "I", &[]),
                &[wreg],
            );
            self.asm.move_result(year, false);
        } else {
            self.asm.move_(year, vreg, false);
        }
        if mode != "date-month" {
            self.invoke(
                "virtual",
                method("Landroid/widget/DatePicker;", "getMonth", "I", &[]),
                &[wreg],
            );
            self.asm.move_result(month, false);
        } else {
            self.asm.move_(month, vreg, false);
            self.asm.const_(vreg, 1);
            self.asm.binop(0x91, month, month, vreg, "sub-int");
        }
        if mode != "date-day" {
            self.invoke(
                "virtual",
                method("Landroid/widget/DatePicker;", "getDayOfMonth", "I", &[]),
                &[wreg],
            );
            self.asm.move_result(day, false);
        } else {
            self.asm.move_(day, vreg, false);
        }
        self.invoke(
            "virtual",
            method(
                "Landroid/widget/DatePicker;",
                "updateDate",
                "V",
                &["I", "I", "I"],
            ),
            &[wreg, year, month, day],
        );
        Ok(())
    }

    fn emit_web_assign(&mut self, wreg: u16, vreg: u16, mode: &str) -> Result<(), CompileError> {
        let settings = self.alloc()?;
        self.invoke(
            "virtual",
            method(
                "Landroid/webkit/WebView;",
                "getSettings",
                "Landroid/webkit/WebSettings;",
                &[],
            ),
            &[wreg],
        );
        self.asm.move_result(settings, true);
        let one = self.alloc()?;
        self.asm.const_(one, 1);
        self.invoke(
            "virtual",
            method(
                "Landroid/webkit/WebSettings;",
                "setJavaScriptEnabled",
                "V",
                &["Z"],
            ),
            &[settings, one],
        );
        if mode == "web-url" {
            self.invoke(
                "virtual",
                method(
                    "Landroid/webkit/WebView;",
                    "loadUrl",
                    "V",
                    &["Ljava/lang/String;"],
                ),
                &[wreg, vreg],
            );
            return Ok(());
        }
        let mime = self.alloc()?;
        self.asm.const_string(mime, "text/html");
        let enc = self.alloc()?;
        self.asm.const_string(enc, "utf-8");
        self.invoke(
            "virtual",
            method(
                "Landroid/webkit/WebView;",
                "loadData",
                "V",
                &[
                    "Ljava/lang/String;",
                    "Ljava/lang/String;",
                    "Ljava/lang/String;",
                ],
            ),
            &[wreg, vreg, mime, enc],
        );
        Ok(())
    }

    fn eval(&mut self, expr: &Expr) -> Result<Val, CompileError> {
        match expr {
            Expr::Literal(expr) => self.eval_literal(expr),
            Expr::Ident(expr) => self.eval_name(expr),
            Expr::Designator(expr) => self.eval_prop(expr),
            Expr::Unary(expr) => self.eval_unary(expr),
            Expr::Binary(expr) => {
                if expr.op == "+" && expr.string_add {
                    return self.eval_strcat(expr);
                }
                if expr.op == "and" || expr.op == "or" {
                    return self.eval_logic(expr);
                }
                if cmp_kind(&expr.op).is_some() {
                    return self.eval_cmp(expr);
                }
                self.eval_arith(expr)
            }
            Expr::Call(expr) => {
                let value = self.eval_call(
                    expr.resolved.as_ref(),
                    &expr.args,
                    expr.form_name.as_deref(),
                    false,
                )?;
                value.ok_or_else(|| CompileError::bare("cannot compile expression"))
            }
        }
    }

    fn eval_literal(&mut self, expr: &Literal) -> Result<Val, CompileError> {
        let reg = self.alloc()?;
        match expr.type_name.as_str() {
            "String" => {
                let Lit::Str(text) = &expr.value else {
                    return Err(CompileError::bare("bad string literal"));
                };
                self.asm.const_string(reg, text);
            }
            "Boolean" => {
                let value = match &expr.value {
                    Lit::Bool(value) => i64::from(*value),
                    Lit::Int(value) => i64::from(*value != 0),
                    _ => return Err(CompileError::bare("bad boolean literal")),
                };
                self.asm.const_(reg, value);
            }
            _ => {
                let value = match &expr.value {
                    Lit::Int(value) => *value,
                    Lit::Bool(value) => i64::from(*value),
                    _ => return Err(CompileError::bare("bad integer literal")),
                };
                self.asm.const_(reg, value);
            }
        }
        Ok((reg, expr.type_name.clone()))
    }

    fn eval_name(&mut self, expr: &Ident) -> Result<Val, CompileError> {
        match expr.resolved.clone() {
            Some(Resolved::Widget(widget)) => {
                let reg = self.alloc()?;
                self.load_widget(reg, &widget)?;
                Ok((reg, "View".into()))
            }
            Some(Resolved::Slot { ty, .. }) => {
                let (src, slot_ty) = self.frame.get(&expr.name.to_lowercase())?;
                let reg = self.alloc()?;
                self.asm.move_(reg, src, is_obj(&slot_ty));
                let out = if slot_ty.is_empty() { ty } else { slot_ty };
                Ok((reg, out))
            }
            _ => Err(CompileError::bare("cannot compile expression")),
        }
    }

    fn eval_prop(&mut self, expr: &Designator) -> Result<Val, CompileError> {
        let Resolved::Prop { actual, widget } = expr
            .resolved
            .clone()
            .ok_or_else(|| CompileError::bare("cannot compile expression"))?
        else {
            return Err(CompileError::bare("cannot compile expression"));
        };
        let spec = getter_ref(&actual)?;
        let wreg = self.alloc()?;
        self.load_widget(wreg, &widget)?;
        if comp_str(&widget.comp, "type")? == "Timer" {
            let tick = tick_desc(&self.package);
            let (name, ret, ty) = if actual.eq_ignore_ascii_case("Enabled") {
                ("enabled", "Z", "Boolean")
            } else if actual.eq_ignore_ascii_case("Interval") {
                ("interval", "I", "Integer")
            } else {
                return Err(CompileError::bare(format!("Timer has no property {actual}")));
            };
            self.invoke("virtual", method(&tick, name, ret, &[]), &[wreg]);
            self.asm.move_result(wreg, false);
            return Ok((wreg, ty.into()));
        }
        if spec.mode == "items" || spec.mode == "details" {
            let rt = class_desc(&self.package, "DroideRt");
            let name = if spec.mode == "items" {
                "itemsOf"
            } else {
                "detailsOf"
            };
            self.invoke(
                "static",
                method(&rt, name, "Ljava/lang/String;", &["Landroid/view/View;"]),
                &[wreg],
            );
            self.asm.move_result(wreg, true);
            return Ok((wreg, "String".into()));
        }
        self.invoke("virtual", dex_method(spec), &[wreg]);
        if spec.mode == "text" {
            self.asm.move_result(wreg, true);
            self.invoke(
                "virtual",
                method("Ljava/lang/Object;", "toString", "Ljava/lang/String;", &[]),
                &[wreg],
            );
            self.asm.move_result(wreg, true);
            return Ok((wreg, "String".into()));
        }
        self.asm.move_result(wreg, false);
        if spec.mode == "month" {
            // DatePicker.getMonth is 0–11. Droide months are 1–12.
            let one = self.alloc()?;
            self.asm.const_(one, 1);
            self.asm.binop(0x90, wreg, wreg, one, "add-int");
            self.release_to(wreg + 1);
            return Ok((wreg, "Integer".into()));
        }
        if spec.mode == "float" {
            self.asm.unop(0x87, wreg, wreg, "float-to-int");
            return Ok((wreg, "Integer".into()));
        }
        if spec.mode == "vis" {
            let zero = self.alloc()?;
            self.asm.const_(zero, 0);
            let yes = self.fresh();
            let end = self.fresh();
            self.zero_except(&[wreg, zero]);
            self.asm.if_cmp("eq", wreg, zero, &yes);
            self.asm.const_(wreg, 0);
            self.asm.const_(zero, 0);
            self.asm.goto(&end);
            self.asm.label(&yes);
            self.asm.const_(wreg, 1);
            self.asm.const_(zero, 0);
            self.asm.label(&end);
            self.release_to(wreg + 1);
            return Ok((wreg, "Boolean".into()));
        }
        let ty = if spec.mode == "bool" {
            "Boolean"
        } else {
            "Integer"
        };
        Ok((wreg, ty.into()))
    }

    fn eval_unary(&mut self, expr: &Unary) -> Result<Val, CompileError> {
        let (reg, ty) = self.eval(&expr.expr)?;
        if expr.op == "+" {
            return Ok((reg, ty));
        }
        if expr.op == "-" {
            self.asm.unop(0x7B, reg, reg, "neg-int");
            return Ok((reg, "Integer".into()));
        }
        let yes = self.fresh();
        let end = self.fresh();
        self.zero_except(&[reg]);
        self.asm.if_zero("eqz", reg, &yes);
        self.asm.const_(reg, 0);
        self.asm.goto(&end);
        self.asm.label(&yes);
        self.asm.const_(reg, 1);
        self.asm.label(&end);
        Ok((reg, "Boolean".into()))
    }

    fn eval_arith(&mut self, expr: &Binary) -> Result<Val, CompileError> {
        let (left, _) = self.eval(&expr.left)?;
        let (right, _) = self.eval(&expr.right)?;
        let (op, name) =
            int_bin(&expr.op).ok_or_else(|| CompileError::bare("cannot compile expression"))?;
        self.asm.binop(op, left, left, right, name);
        self.release_to(left + 1);
        Ok((left, "Integer".into()))
    }

    fn eval_logic(&mut self, expr: &Binary) -> Result<Val, CompileError> {
        let (reg, _) = self.eval(&expr.left)?;
        let done = self.fresh();
        self.zero_except(&[reg]);
        if expr.op == "and" {
            self.asm.if_zero("eqz", reg, &done);
        } else {
            self.asm.if_zero("nez", reg, &done);
        }
        self.sp = reg;
        self.eval(&expr.right)?;
        self.release_to(reg + 1);
        self.asm.label(&done);
        self.sp = reg + 1;
        Ok((reg, "Boolean".into()))
    }

    fn eval_cmp(&mut self, expr: &Binary) -> Result<Val, CompileError> {
        let (left, lt) = self.eval(&expr.left)?;
        let (right, rt) = self.eval(&expr.right)?;
        if lt == "String" || rt == "String" {
            self.cmp_string(&expr.op, left, right)
        } else {
            self.cmp_int(&expr.op, left, right)
        }
    }

    fn cmp_int(&mut self, op: &str, left: u16, right: u16) -> Result<Val, CompileError> {
        let kind = cmp_kind(op).ok_or_else(|| CompileError::bare("cannot compile expression"))?;
        let yes = self.fresh();
        let end = self.fresh();
        self.zero_except(&[left, right]);
        self.asm.if_cmp(kind, left, right, &yes);
        self.asm.const_(left, 0);
        self.asm.const_(right, 0);
        self.asm.goto(&end);
        self.asm.label(&yes);
        self.asm.const_(left, 1);
        self.asm.const_(right, 0);
        self.asm.label(&end);
        self.release_to(left + 1);
        Ok((left, "Boolean".into()))
    }

    fn cmp_string(&mut self, op: &str, left: u16, right: u16) -> Result<Val, CompileError> {
        if op == "=" || op == "<>" {
            self.invoke(
                "virtual",
                method("Ljava/lang/String;", "equals", "Z", &["Ljava/lang/Object;"]),
                &[left, right],
            );
            self.asm.move_result(left, false);
            self.asm.const_(right, 0);
            self.release_to(left + 1);
            if op == "<>" {
                self.invert(left);
            }
            return Ok((left, "Boolean".into()));
        }
        self.invoke(
            "virtual",
            method(
                "Ljava/lang/String;",
                "compareTo",
                "I",
                &["Ljava/lang/String;"],
            ),
            &[left, right],
        );
        self.asm.move_result(left, false);
        self.asm.const_(right, 0);
        self.cmp_int(op, left, right)
    }

    fn invert(&mut self, reg: u16) {
        let yes = self.fresh();
        let end = self.fresh();
        self.zero_except(&[reg]);
        self.asm.if_zero("eqz", reg, &yes);
        self.asm.const_(reg, 0);
        self.asm.goto(&end);
        self.asm.label(&yes);
        self.asm.const_(reg, 1);
        self.asm.label(&end);
    }

    fn eval_strcat(&mut self, expr: &Binary) -> Result<Val, CompileError> {
        let mut parts = Vec::new();
        flatten_strcat(&Expr::Binary(expr.clone()), &mut parts);
        let sb = self.alloc()?;
        self.asm.new_instance(sb, "Ljava/lang/StringBuilder;");
        self.invoke(
            "direct",
            method("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
            &[sb],
        );
        for part in &parts {
            let (reg, ty) = self.eval(part)?;
            let proto = if ty == "Integer" {
                "I"
            } else {
                "Ljava/lang/String;"
            };
            self.invoke(
                "virtual",
                method(
                    "Ljava/lang/StringBuilder;",
                    "append",
                    "Ljava/lang/StringBuilder;",
                    &[proto],
                ),
                &[sb, reg],
            );
            self.asm.move_result(sb, true);
            self.release_to(sb + 1);
        }
        self.invoke(
            "virtual",
            method(
                "Ljava/lang/StringBuilder;",
                "toString",
                "Ljava/lang/String;",
                &[],
            ),
            &[sb],
        );
        self.asm.move_result(sb, true);
        Ok((sb, "String".into()))
    }

    fn eval_call(
        &mut self,
        resolved: Option<&Resolved>,
        args: &[Expr],
        form_name: Option<&str>,
        as_stmt: bool,
    ) -> Result<Option<Val>, CompileError> {
        let resolved = resolved
            .ok_or_else(|| CompileError::bare("cannot compile expression"))?
            .clone();
        let start = self.sp;
        // The form name is a const-class, not a live string. Leaving that
        // register free is what lets four extras fit beside the intent.
        let skip_form = matches!(
            (&resolved, args.first()),
            (Resolved::Builtin(name), Some(Expr::Literal(lit)))
                if name == "OpenForm" && matches!(lit.value, Lit::Str(_))
        );
        let mut arg_regs = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            if skip_form && index == 0 {
                continue;
            }
            let (reg, _) = self.eval(arg)?;
            arg_regs.push(reg);
        }
        match resolved {
            Resolved::Builtin(name) => {
                self.emit_builtin(&name, args, form_name, &arg_regs, start, as_stmt)
            }
            Resolved::Native(native) => {
                let key = MethodRef {
                    owner: self.native_desc.clone(),
                    name: native.name,
                    ret: "I".into(),
                    params: vec!["I".to_string(); native.params.len()],
                };
                self.finish_call(
                    "static",
                    key,
                    &arg_regs,
                    start,
                    Some("Integer"),
                    as_stmt,
                    false,
                )
            }
            Resolved::Routine(target) => {
                let owner = class_desc(&self.package, &target.unit);
                let ret_desc = match &target.ret {
                    None => "V".to_string(),
                    Some(ty) => type_desc(ty)?,
                };
                let params = target
                    .params
                    .iter()
                    .map(|ty| type_desc(ty))
                    .collect::<Result<Vec<_>, _>>()?;
                let key = MethodRef {
                    owner,
                    name: target.name,
                    ret: ret_desc,
                    params,
                };
                let kind = if target.is_static {
                    "static"
                } else {
                    "virtual"
                };
                self.finish_call(
                    kind,
                    key,
                    &arg_regs,
                    start,
                    target.ret.as_deref(),
                    as_stmt,
                    !target.is_static,
                )
            }
            _ => Err(CompileError::bare("cannot compile expression")),
        }
    }

    fn finish_call(
        &mut self,
        kind: &str,
        key: MethodRef,
        arg_regs: &[u16],
        start: u16,
        ret: Option<&str>,
        as_stmt: bool,
        instance: bool,
    ) -> Result<Option<Val>, CompileError> {
        let mut regs = Vec::new();
        if instance {
            let this = self
                .frame
                .this
                .ok_or_else(|| CompileError::bare("instance call outside a form"))?;
            regs.push(this);
        }
        regs.extend_from_slice(arg_regs);
        self.invoke(kind, key, &regs);
        let Some(ret) = ret else {
            self.release_to(start);
            return Ok(None);
        };
        let dest = if let Some(&reg) = arg_regs.first() {
            reg
        } else {
            self.alloc()?
        };
        self.asm.move_result(dest, is_obj(ret));
        if as_stmt {
            self.release_to(start);
            return Ok(None);
        }
        if dest != start {
            self.asm.move_(start, dest, is_obj(ret));
        }
        self.release_to(start + 1);
        self.sp = start + 1;
        Ok(Some((start, ret.to_string())))
    }

    fn new_builder(&mut self, this: u16) -> Result<u16, CompileError> {
        let builder = self.alloc()?;
        self.asm
            .new_instance(builder, "Landroid/app/AlertDialog$Builder;");
        self.invoke(
            "direct",
            method(
                "Landroid/app/AlertDialog$Builder;",
                "<init>",
                "V",
                &["Landroid/content/Context;"],
            ),
            &[builder, this],
        );
        Ok(builder)
    }

    fn alert_message(&mut self, builder: u16, text: u16) {
        self.invoke(
            "virtual",
            method(
                "Landroid/app/AlertDialog$Builder;",
                "setMessage",
                "Landroid/app/AlertDialog$Builder;",
                &["Ljava/lang/CharSequence;"],
            ),
            &[builder, text],
        );
        self.asm.move_result(builder, true);
    }

    fn alert_button(&mut self, name: &str, builder: u16, label: u16, listener: u16) {
        self.invoke(
            "virtual",
            method(
                "Landroid/app/AlertDialog$Builder;",
                name,
                "Landroid/app/AlertDialog$Builder;",
                &[
                    "Ljava/lang/CharSequence;",
                    "Landroid/content/DialogInterface$OnClickListener;",
                ],
            ),
            &[builder, label, listener],
        );
        self.asm.move_result(builder, true);
    }

    fn put_extra(&mut self, intent: u16, key: u16, value: u16, value_desc: &str) {
        self.invoke(
            "virtual",
            method(
                "Landroid/content/Intent;",
                "putExtra",
                "Landroid/content/Intent;",
                &["Ljava/lang/String;", value_desc],
            ),
            &[intent, key, value],
        );
        self.asm.move_result(intent, true);
    }

    fn alert_show(&mut self, builder: u16, dest: u16) {
        self.invoke(
            "virtual",
            method(
                "Landroid/app/AlertDialog$Builder;",
                "show",
                "Landroid/app/AlertDialog;",
                &[],
            ),
            &[builder],
        );
        self.asm.move_result(dest, true);
    }

    fn emit_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        form_name: Option<&str>,
        arg_regs: &[u16],
        start: u16,
        as_stmt: bool,
    ) -> Result<Option<Val>, CompileError> {
        match name {
            "Toast" => {
                let dur = self.alloc()?;
                self.asm.const_(dur, 0);
                self.invoke(
                    "static",
                    method(
                        "Landroid/widget/Toast;",
                        "makeText",
                        "Landroid/widget/Toast;",
                        &["Landroid/content/Context;", "Ljava/lang/CharSequence;", "I"],
                    ),
                    &[
                        self.frame
                            .this
                            .ok_or_else(|| CompileError::bare("Toast outside a form"))?,
                        arg_regs[0],
                        dur,
                    ],
                );
                self.asm.move_result(arg_regs[0], true);
                self.invoke(
                    "virtual",
                    method("Landroid/widget/Toast;", "show", "V", &[]),
                    &[arg_regs[0]],
                );
                self.release_to(start);
                Ok(None)
            }
            "Message" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("Message outside a form"))?;
                let builder = self.new_builder(this)?;
                self.alert_message(builder, arg_regs[0]);
                let ok = self.alloc()?;
                self.asm.const_string(ok, "OK");
                let none = self.alloc()?;
                self.asm.const_(none, 0);
                self.alert_button("setPositiveButton", builder, ok, none);
                self.alert_show(builder, ok);
                self.release_to(start);
                Ok(None)
            }
            "Question" => {
                let handler = form_name
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| CompileError::bare("Question needs a procedure name"))?;
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("Question outside a form"))?;
                // The procedure name is fixed at check time. Drop its register
                // so the builder, the label, and the listener fit in scratch.
                if self.sp > start + 1 && arg_regs.get(1).copied() == Some(self.sp - 1) {
                    self.release_to(self.sp - 1);
                }
                let builder = self.new_builder(this)?;
                self.alert_message(builder, arg_regs[0]);
                let yes = self.alloc()?;
                self.asm.const_string(yes, "Yes");
                let form_desc = self.owner.clone();
                let ldesc = format!("{}$Ask${handler};", form_desc.trim_end_matches(';'));
                let listen = self.alloc()?;
                self.asm.new_instance(listen, &ldesc);
                self.invoke(
                    "direct",
                    method(&ldesc, "<init>", "V", &[form_desc.as_str()]),
                    &[listen, this],
                );
                self.alert_button("setPositiveButton", builder, yes, listen);
                self.release_to(builder + 1);
                let no = self.alloc()?;
                self.asm.const_string(no, "No");
                let none = self.alloc()?;
                self.asm.const_(none, 0);
                self.alert_button("setNegativeButton", builder, no, none);
                self.alert_show(builder, no);
                self.release_to(start);
                Ok(None)
            }
            "Log" => {
                let tag = self.alloc()?;
                self.asm.const_string(tag, "Droide");
                self.invoke(
                    "static",
                    method(
                        "Landroid/util/Log;",
                        "i",
                        "I",
                        &["Ljava/lang/String;", "Ljava/lang/String;"],
                    ),
                    &[tag, arg_regs[0]],
                );
                self.asm.move_result(tag, false);
                self.release_to(start);
                Ok(None)
            }
            "Finish" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("Finish outside a form"))?;
                self.invoke(
                    "virtual",
                    method("Landroid/app/Activity;", "finish", "V", &[]),
                    &[this],
                );
                self.release_to(start);
                Ok(None)
            }
            "Page" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("Page outside a form"))?;
                let rt = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(&rt, "page", "V", &["Landroid/app/Activity;", "I"]),
                    &[this, arg_regs[0]],
                );
                self.release_to(start);
                Ok(None)
            }
            "Grow" => {
                let rt = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(&rt, "grow", "V", &["Landroid/view/View;"]),
                    &[arg_regs[0]],
                );
                self.release_to(start);
                Ok(None)
            }
            "Place" => {
                let rt = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(&rt, "place", "V", &["Landroid/view/View;", "I", "I"]),
                    &[arg_regs[0], arg_regs[1], arg_regs[2]],
                );
                self.release_to(start);
                Ok(None)
            }
            "OpenForm" => {
                let form_name = open_form_name(form_name, args)?;
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("OpenForm outside a form"))?;
                let skipped = matches!(
                    args.first(),
                    Some(Expr::Literal(lit)) if matches!(lit.value, Lit::Str(_))
                );
                let extra_regs: Vec<u16> = if skipped {
                    arg_regs.to_vec()
                } else {
                    arg_regs.iter().skip(1).copied().collect()
                };
                let extra_exprs: Vec<&Expr> = args.iter().skip(1).collect();
                if extra_regs.len() != extra_exprs.len() {
                    return Err(CompileError::bare("OpenForm needs a form name"));
                }
                let intent = self.alloc()?;
                self.asm.new_instance(intent, "Landroid/content/Intent;");
                self.invoke(
                    "direct",
                    method("Landroid/content/Intent;", "<init>", "V", &[]),
                    &[intent],
                );
                // After setClass the class register is free and becomes the extra key.
                let key = self.alloc()?;
                self.asm
                    .const_class(key, &class_desc(&self.package, &form_name));
                self.invoke(
                    "virtual",
                    method(
                        "Landroid/content/Intent;",
                        "setClass",
                        "Landroid/content/Intent;",
                        &["Landroid/content/Context;", "Ljava/lang/Class;"],
                    ),
                    &[intent, this, key],
                );
                self.asm.move_result(intent, true);
                for (index, (reg, expr)) in extra_regs.iter().zip(extra_exprs.iter()).enumerate() {
                    let (letter, desc) = extra_put(expr)?;
                    // Value first, then the type tag, so a torn write looks missing.
                    self.asm.const_string(key, &format!("a{index}"));
                    self.put_extra(intent, key, *reg, desc);
                    self.asm.const_string(*reg, letter);
                    self.asm.const_string(key, &format!("t{index}"));
                    self.put_extra(intent, key, *reg, "Ljava/lang/String;");
                }
                self.invoke(
                    "virtual",
                    method(
                        "Landroid/app/Activity;",
                        "startActivity",
                        "V",
                        &["Landroid/content/Intent;"],
                    ),
                    &[this, intent],
                );
                self.release_to(start);
                Ok(None)
            }
            "Arg" | "ArgInt" | "ArgBool" => {
                let (runtime, ret_desc, ret_ty, obj) = match name {
                    "Arg" => ("arg", "Ljava/lang/String;", "String", true),
                    "ArgInt" => ("argInt", "I", "Integer", false),
                    "ArgBool" => ("argBool", "Z", "Boolean", false),
                    _ => unreachable!(),
                };
                let this = self.frame.this.ok_or_else(|| {
                    CompileError::bare(format!("{name} outside a form"))
                })?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        runtime,
                        ret_desc,
                        &["Landroid/app/Activity;", "I"],
                    ),
                    &[this, arg_regs[0]],
                );
                self.asm.move_result(start, obj);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, ret_ty.into())))
                }
            }
            "IntToStr" => {
                self.invoke(
                    "static",
                    method(
                        "Ljava/lang/String;",
                        "valueOf",
                        "Ljava/lang/String;",
                        &["I"],
                    ),
                    &[arg_regs[0]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "StrToInt" => {
                self.invoke(
                    "static",
                    method(
                        "Ljava/lang/Integer;",
                        "parseInt",
                        "I",
                        &["Ljava/lang/String;"],
                    ),
                    &[arg_regs[0]],
                );
                self.asm.move_result(start, false);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "Integer".into())))
                }
            }
            "IsInt" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(&owner, "isInt", "Z", &["Ljava/lang/String;"]),
                    &[arg_regs[0]],
                );
                self.asm.move_result(start, false);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "Boolean".into())))
                }
            }
            "Length" => {
                self.invoke(
                    "virtual",
                    method("Ljava/lang/String;", "length", "I", &[]),
                    &[arg_regs[0]],
                );
                self.asm.move_result(start, false);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "Integer".into())))
                }
            }
            "WriteText" => {
                let this = self.frame.this.ok_or_else(|| {
                    CompileError::bare("WriteText outside a form")
                })?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "writeText",
                        "V",
                        &[
                            "Landroid/content/Context;",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                        ],
                    ),
                    &[this, arg_regs[0], arg_regs[1]],
                );
                self.release_to(start);
                Ok(None)
            }
            "ReadText" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("ReadText outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "readText",
                        "Ljava/lang/String;",
                        &["Landroid/content/Context;", "Ljava/lang/String;"],
                    ),
                    &[this, arg_regs[0]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "Where" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("Where outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(&owner, "where", "Ljava/lang/String;", &[
                        "Landroid/content/Context;",
                    ]),
                    &[this],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "OpenCamera" => {
                let this = self.frame.this.ok_or_else(|| {
                    CompileError::bare("OpenCamera outside a form")
                })?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(&owner, "openCamera", "V", &["Landroid/content/Context;"]),
                    &[this],
                );
                self.release_to(start);
                Ok(None)
            }
            "TakePhoto" | "OpenDocument" => self.emit_activity_result(name, form_name, start),
            "Share" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("Share outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "share",
                        "V",
                        &["Landroid/content/Context;", "Ljava/lang/String;"],
                    ),
                    &[this, arg_regs[0]],
                );
                self.release_to(start);
                Ok(None)
            }
            "CopyText" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("CopyText outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "copyText",
                        "V",
                        &["Landroid/content/Context;", "Ljava/lang/String;"],
                    ),
                    &[this, arg_regs[0]],
                );
                self.release_to(start);
                Ok(None)
            }
            "LineCount" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(&owner, "lineCount", "I", &["Ljava/lang/String;"]),
                    &[arg_regs[0]],
                );
                self.asm.move_result(start, false);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "Integer".into())))
                }
            }
            "ToggleLine" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "toggleLine",
                        "Ljava/lang/String;",
                        &["Ljava/lang/String;", "Ljava/lang/String;"],
                    ),
                    &[arg_regs[0], arg_regs[1]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "DropNames" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "dropNames",
                        "Ljava/lang/String;",
                        &["Ljava/lang/String;", "Ljava/lang/String;"],
                    ),
                    &[arg_regs[0], arg_regs[1]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "DropParallel" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "dropParallel",
                        "Ljava/lang/String;",
                        &[
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                        ],
                    ),
                    &[arg_regs[0], arg_regs[1], arg_regs[2]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "TakeParallel" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "takeParallel",
                        "Ljava/lang/String;",
                        &[
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                        ],
                    ),
                    &[arg_regs[0], arg_regs[1], arg_regs[2]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "JoinNotes" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("JoinNotes outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "joinNotes",
                        "Ljava/lang/String;",
                        &["Landroid/content/Context;", "Ljava/lang/String;"],
                    ),
                    &[this, arg_regs[0]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "SameDays" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "sameDays",
                        "Ljava/lang/String;",
                        &["Ljava/lang/String;", "Ljava/lang/String;"],
                    ),
                    &[arg_regs[0], arg_regs[1]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "JournalItems" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "journalItems",
                        "Ljava/lang/String;",
                        &["Ljava/lang/String;", "Ljava/lang/String;"],
                    ),
                    &[arg_regs[0], arg_regs[1]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "DatePart" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(&owner, "datePart", "I", &["Ljava/lang/String;", "I"]),
                    &[arg_regs[0], arg_regs[1]],
                );
                self.asm.move_result(start, false);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "Integer".into())))
                }
            }
            "Weave" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "weave",
                        "Ljava/lang/String;",
                        &[
                            "I",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                        ],
                    ),
                    &[
                        arg_regs[0],
                        arg_regs[1],
                        arg_regs[2],
                        arg_regs[3],
                        arg_regs[4],
                    ],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "AgeDay" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(&owner, "ageDay", "Ljava/lang/String;", &["I", "I", "I", "I", "I"]),
                    &[
                        arg_regs[0],
                        arg_regs[1],
                        arg_regs[2],
                        arg_regs[3],
                        arg_regs[4],
                    ],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "ClearAges" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("ClearAges outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(&owner, "clearAges", "V", &["Landroid/content/Context;"]),
                    &[this],
                );
                self.release_to(start);
                Ok(None)
            }
            "PlaceAge" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("PlaceAge outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "placeAge",
                        "V",
                        &[
                            "Landroid/content/Context;",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                        ],
                    ),
                    &[this, arg_regs[0], arg_regs[1], arg_regs[2]],
                );
                self.release_to(start);
                Ok(None)
            }
            "JournalDetails" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "journalDetails",
                        "Ljava/lang/String;",
                        &[
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                        ],
                    ),
                    &[arg_regs[0], arg_regs[1], arg_regs[2]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "LineIndex" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "lineIndex",
                        "I",
                        &["Ljava/lang/String;", "Ljava/lang/String;"],
                    ),
                    &[arg_regs[0], arg_regs[1]],
                );
                self.asm.move_result(start, false);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "Integer".into())))
                }
            }
            "LineAt" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "lineAt",
                        "Ljava/lang/String;",
                        &["Ljava/lang/String;", "I"],
                    ),
                    &[arg_regs[0], arg_regs[1]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "ReplaceAt" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "replaceAt",
                        "Ljava/lang/String;",
                        &["Ljava/lang/String;", "I", "Ljava/lang/String;"],
                    ),
                    &[arg_regs[0], arg_regs[1], arg_regs[2]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "DropAt" => {
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "dropAt",
                        "Ljava/lang/String;",
                        &["Ljava/lang/String;", "I"],
                    ),
                    &[arg_regs[0], arg_regs[1]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "Today" | "Time" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare(format!("{name} outside a form")))?;
                let owner = class_desc(&self.package, "DroideRt");
                let method_name = if name == "Today" { "today" } else { "time" };
                self.invoke(
                    "static",
                    method(
                        &owner,
                        method_name,
                        "Ljava/lang/String;",
                        &["Landroid/content/Context;"],
                    ),
                    &[this],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "PasteText" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("PasteText outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "pasteText",
                        "Ljava/lang/String;",
                        &["Landroid/content/Context;"],
                    ),
                    &[this],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "Search" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("Search outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "search",
                        "Ljava/lang/String;",
                        &["Landroid/content/Context;", "Ljava/lang/String;"],
                    ),
                    &[this, arg_regs[0]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "Lookup" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("Lookup outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "lookup",
                        "Ljava/lang/String;",
                        &["Landroid/content/Context;", "Ljava/lang/String;"],
                    ),
                    &[this, arg_regs[0]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            "Ingest" => {
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("Ingest outside a form"))?;
                let owner = class_desc(&self.package, "DroideRt");
                self.invoke(
                    "static",
                    method(
                        &owner,
                        "ingest",
                        "Ljava/lang/String;",
                        &[
                            "Landroid/content/Context;",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                            "Ljava/lang/String;",
                        ],
                    ),
                    &[this, arg_regs[0], arg_regs[1], arg_regs[2]],
                );
                self.asm.move_result(start, true);
                self.release_to(start + 1);
                self.sp = start + 1;
                if as_stmt {
                    Ok(None)
                } else {
                    Ok(Some((start, "String".into())))
                }
            }
            _ => Err(CompileError::bare(format!("unknown builtin {name}"))),
        }
    }
}

fn activity_init() -> MethodModel {
    let mut asm = Asm::new();
    asm.invoke(
        "direct",
        method("Landroid/app/Activity;", "<init>", "V", &[]),
        &[0],
    );
    asm.ret(0, "void");
    MethodModel::new(
        "<init>",
        "V",
        Vec::new(),
        ACC_PUBLIC | ACC_CONSTRUCTOR,
        Some(asm),
        1,
        1,
        1,
    )
}

fn emit_const_prop(
    asm: &mut Asm,
    wreg: u16,
    prop: &str,
    value: &Value,
) -> Result<(), CompileError> {
    let spec = setter_ref(prop)?;
    let key = dex_method(spec);
    match spec.mode {
        "string" => {
            let text = if value.is_null() {
                String::new()
            } else {
                py_str(value)
            };
            asm.const_string(4, &text);
            asm.invoke("virtual", key, &[wreg, 4]);
        }
        "float" => {
            asm.const_(4, json_int(value)?);
            asm.unop(0x82, 4, 4, "int-to-float");
            asm.invoke("virtual", key, &[wreg, 4]);
        }
        "int" => {
            let number = if prop == "Color" || prop == "TextColor" {
                parse_color(value).map_err(CompileError::bare)? as i64
            } else {
                json_int(value)?
            };
            asm.const_(4, number);
            asm.invoke("virtual", key, &[wreg, 4]);
        }
        "bool" => {
            asm.const_(4, if truthy(value) { 1 } else { 0 });
            asm.invoke("virtual", key, &[wreg, 4]);
        }
        "stack" => {
            let on = truthy(value);
            asm.const_(4, if on { 1 } else { 0 });
            asm.invoke(
                "virtual",
                method(
                    "Landroid/widget/AbsListView;",
                    "setStackFromBottom",
                    "V",
                    &["Z"],
                ),
                &[wreg, 4],
            );
            // ALWAYS_SCROLL is 2. The newest row stays against the bottom edge.
            asm.const_(4, if on { 2 } else { 0 });
            asm.invoke(
                "virtual",
                method(
                    "Landroid/widget/AbsListView;",
                    "setTranscriptMode",
                    "V",
                    &["I"],
                ),
                &[wreg, 4],
            );
        }
        "vis" => {
            asm.const_(4, if truthy(value) { 0 } else { 8 });
            asm.invoke("virtual", key, &[wreg, 4]);
        }
        "clock" => {
            let text = if value.is_null() {
                String::new()
            } else {
                py_str(value)
            };
            asm.const_string(4, &text);
            let params = &["Ljava/lang/CharSequence;"];
            asm.invoke(
                "virtual",
                method("Landroid/widget/TextClock;", "setFormat12Hour", "V", params),
                &[wreg, 4],
            );
            asm.invoke(
                "virtual",
                method("Landroid/widget/TextClock;", "setFormat24Hour", "V", params),
                &[wreg, 4],
            );
        }
        "boxbool" => {
            asm.const_(4, if truthy(value) { 1 } else { 0 });
            asm.invoke(
                "static",
                method(
                    "Ljava/lang/Boolean;",
                    "valueOf",
                    "Ljava/lang/Boolean;",
                    &["Z"],
                ),
                &[4],
            );
            asm.move_result(4, true);
            asm.invoke("virtual", key, &[wreg, 4]);
        }
        "date-year" | "date-month" | "date-day" => {}
        _ => {}
    }
    Ok(())
}

fn change_hook(kind: &str) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match kind {
        "Edit" | "Memo" | "Number" | "Password" | "Email" | "Phone" => (
            "Landroid/widget/TextView;",
            "addTextChangedListener",
            "Landroid/text/TextWatcher;",
        ),
        "Check" | "Radio" | "Switch" | "Toggle" => (
            "Landroid/widget/CompoundButton;",
            "setOnCheckedChangeListener",
            "Landroid/widget/CompoundButton$OnCheckedChangeListener;",
        ),
        "Seek" => (
            "Landroid/widget/SeekBar;",
            "setOnSeekBarChangeListener",
            "Landroid/widget/SeekBar$OnSeekBarChangeListener;",
        ),
        "Rating" => (
            "Landroid/widget/RatingBar;",
            "setOnRatingBarChangeListener",
            "Landroid/widget/RatingBar$OnRatingBarChangeListener;",
        ),
        "Picker" => (
            "Landroid/widget/NumberPicker;",
            "setOnValueChangedListener",
            "Landroid/widget/NumberPicker$OnValueChangeListener;",
        ),
        "Time" => (
            "Landroid/widget/TimePicker;",
            "setOnTimeChangedListener",
            "Landroid/widget/TimePicker$OnTimeChangedListener;",
        ),
        _ => return None,
    })
}

fn change_iface(kind: &str) -> &'static str {
    if let Some((_, _, iface)) = change_hook(kind) {
        return iface;
    }
    if kind == "Date" {
        "Landroid/widget/DatePicker$OnDateChangedListener;"
    } else {
        "Ljava/lang/Object;"
    }
}

fn empty_method(cls: &mut ClassModel, name: &str, params: &[&str], registers: u16, ins: u16) {
    let mut asm = Asm::new();
    asm.ret(0, "void");
    cls.method(MethodModel::new(
        name,
        "V",
        params.iter().map(|param| (*param).to_string()).collect(),
        ACC_PUBLIC,
        Some(asm),
        registers,
        ins,
        1,
    ));
}

/// Fires when one Return is typed at the end of the field. The text passed
/// along is the field without that Return. A paste, or a Return in the middle, stays put.
fn emit_enter_watcher(
    cls: &mut ClassModel,
    desc: &str,
    form_desc: &str,
    rec: &RoutineInfo,
) -> Result<(), CompileError> {
    empty_method(
        cls,
        "beforeTextChanged",
        &["Ljava/lang/CharSequence;", "I", "I", "I"],
        5,
        5,
    );
    empty_method(cls, "afterTextChanged", &["Landroid/text/Editable;"], 4, 2);
    let mut asm = Asm::new();
    // registers 10, ins 5. v5 this, v6 the text, v7 start, v8 replaced, v9 count.
    // Each failed check returns on its own, so a string never joins an int.
    asm.const_(0, 1);
    asm.if_cmp("eq", 9, 0, "enter_one");
    asm.ret(0, "void");
    asm.label("enter_one");
    int_zero(&mut asm, 0);
    asm.if_cmp("eq", 8, 0, "enter_typed");
    asm.ret(0, "void");
    asm.label("enter_typed");
    asm.invoke(
        "interface",
        method("Ljava/lang/CharSequence;", "charAt", "C", &["I"]),
        &[6, 7],
    );
    asm.move_result(0, false);
    asm.const_(1, 10);
    asm.if_cmp("eq", 0, 1, "enter_nl");
    asm.ret(0, "void");
    asm.label("enter_nl");
    asm.invoke(
        "virtual",
        method("Ljava/lang/Object;", "toString", "Ljava/lang/String;", &[]),
        &[6],
    );
    asm.move_result(1, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[1],
    );
    asm.move_result(2, false);
    asm.const_(3, 1);
    asm.binop(0x90, 3, 7, 3, "add-int");
    asm.if_cmp("eq", 3, 2, "enter_end");
    asm.ret(0, "void");
    asm.label("enter_end");
    int_zero(&mut asm, 0);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I", "I"]),
        &[1, 0, 7],
    );
    asm.move_result(4, true);
    asm.iget_obj(0, 5, field(desc, "this$0", form_desc));
    let key = handler_key(form_desc, rec)?;
    let outs = match rec.params.first().map(String::as_str) {
        None => {
            asm.invoke("virtual", key, &[0]);
            3
        }
        Some("String") => {
            asm.invoke("virtual", key, &[0, 4]);
            3
        }
        Some(other) => {
            return Err(CompileError::bare(format!(
                "OnEnter parameter {other} is not supported"
            )));
        }
    };
    asm.ret(0, "void");
    cls.method(MethodModel::new(
        "onTextChanged",
        "V",
        vec![
            "Ljava/lang/CharSequence;".into(),
            "I".into(),
            "I".into(),
            "I".into(),
        ],
        ACC_PUBLIC,
        Some(asm),
        10,
        5,
        outs,
    ));
    Ok(())
}

fn emit_text_watcher(
    cls: &mut ClassModel,
    desc: &str,
    form_desc: &str,
    rec: &RoutineInfo,
) -> Result<(), CompileError> {
    empty_method(
        cls,
        "beforeTextChanged",
        &["Ljava/lang/CharSequence;", "I", "I", "I"],
        5,
        5,
    );
    empty_method(
        cls,
        "onTextChanged",
        &["Ljava/lang/CharSequence;", "I", "I", "I"],
        5,
        5,
    );
    let mut asm = Asm::new();
    // registers 4, ins 2. v2 is this, v3 is the Editable, v0 is the form, v1 is the text.
    asm.iget_obj(0, 2, field(desc, "this$0", form_desc));
    let key = handler_key(form_desc, rec)?;
    let outs = match rec.params.first().map(String::as_str) {
        None => {
            asm.invoke("virtual", key, &[0]);
            1
        }
        Some("String") => {
            asm.if_zero("eqz", 3, "text_blank");
            asm.invoke(
                "virtual",
                method("Ljava/lang/Object;", "toString", "Ljava/lang/String;", &[]),
                &[3],
            );
            asm.move_result(1, true);
            asm.goto("text_call");
            asm.label("text_blank");
            asm.const_string(1, "");
            asm.label("text_call");
            asm.invoke("virtual", key, &[0, 1]);
            2
        }
        Some(other) => {
            return Err(CompileError::bare(format!(
                "OnChange parameter {other} is not supported"
            )));
        }
    };
    asm.ret(0, "void");
    cls.method(MethodModel::new(
        "afterTextChanged",
        "V",
        vec!["Landroid/text/Editable;".into()],
        ACC_PUBLIC,
        Some(asm),
        4,
        2,
        outs,
    ));
    Ok(())
}

fn emit_checked(
    cls: &mut ClassModel,
    desc: &str,
    form_desc: &str,
    rec: &RoutineInfo,
) -> Result<(), CompileError> {
    let mut asm = Asm::new();
    // registers 4, ins 3. v1 is this, v2 is the button, v3 is the boolean, v0 is the form.
    asm.iget_obj(0, 1, field(desc, "this$0", form_desc));
    let key = handler_key(form_desc, rec)?;
    let outs = match rec.params.first().map(String::as_str) {
        None => {
            asm.invoke("virtual", key, &[0]);
            1
        }
        Some("Boolean") => {
            asm.invoke("virtual", key, &[0, 3]);
            2
        }
        Some(other) => {
            return Err(CompileError::bare(format!(
                "OnChange parameter {other} is not supported"
            )));
        }
    };
    asm.ret(0, "void");
    cls.method(MethodModel::new(
        "onCheckedChanged",
        "V",
        vec!["Landroid/widget/CompoundButton;".into(), "Z".into()],
        ACC_PUBLIC,
        Some(asm),
        4,
        3,
        outs,
    ));
    Ok(())
}

fn emit_seek(
    cls: &mut ClassModel,
    desc: &str,
    form_desc: &str,
    rec: &RoutineInfo,
) -> Result<(), CompileError> {
    empty_method(cls, "onStartTrackingTouch", &["Landroid/widget/SeekBar;"], 2, 2);
    empty_method(cls, "onStopTrackingTouch", &["Landroid/widget/SeekBar;"], 2, 2);
    let mut asm = Asm::new();
    // registers 5, ins 4. v1 this, v2 bar, v3 progress, v4 fromUser, v0 form.
    asm.iget_obj(0, 1, field(desc, "this$0", form_desc));
    let key = handler_key(form_desc, rec)?;
    let outs = match rec.params.first().map(String::as_str) {
        None => {
            asm.invoke("virtual", key, &[0]);
            1
        }
        Some("Integer") => {
            asm.invoke("virtual", key, &[0, 3]);
            2
        }
        Some(other) => {
            return Err(CompileError::bare(format!(
                "OnChange parameter {other} is not supported"
            )));
        }
    };
    asm.ret(0, "void");
    cls.method(MethodModel::new(
        "onProgressChanged",
        "V",
        vec![
            "Landroid/widget/SeekBar;".into(),
            "I".into(),
            "Z".into(),
        ],
        ACC_PUBLIC,
        Some(asm),
        5,
        4,
        outs,
    ));
    Ok(())
}

fn emit_rating(
    cls: &mut ClassModel,
    desc: &str,
    form_desc: &str,
    rec: &RoutineInfo,
) -> Result<(), CompileError> {
    let mut asm = Asm::new();
    // registers 5, ins 4. v1 this, v2 bar, v3 rating float, v4 fromUser, v0 form.
    asm.iget_obj(0, 1, field(desc, "this$0", form_desc));
    let key = handler_key(form_desc, rec)?;
    let outs = match rec.params.first().map(String::as_str) {
        None => {
            asm.invoke("virtual", key, &[0]);
            1
        }
        Some("Integer") => {
            asm.unop(0x87, 3, 3, "float-to-int");
            asm.invoke("virtual", key, &[0, 3]);
            2
        }
        Some(other) => {
            return Err(CompileError::bare(format!(
                "OnChange parameter {other} is not supported"
            )));
        }
    };
    asm.ret(0, "void");
    cls.method(MethodModel::new(
        "onRatingChanged",
        "V",
        vec![
            "Landroid/widget/RatingBar;".into(),
            "F".into(),
            "Z".into(),
        ],
        ACC_PUBLIC,
        Some(asm),
        5,
        4,
        outs,
    ));
    Ok(())
}

fn emit_picker(
    cls: &mut ClassModel,
    desc: &str,
    form_desc: &str,
    rec: &RoutineInfo,
) -> Result<(), CompileError> {
    let mut asm = Asm::new();
    // registers 5, ins 4. v1 this, v2 picker, v3 old value, v4 new value, v0 form.
    asm.iget_obj(0, 1, field(desc, "this$0", form_desc));
    let key = handler_key(form_desc, rec)?;
    let outs = match rec.params.first().map(String::as_str) {
        None => {
            asm.invoke("virtual", key, &[0]);
            1
        }
        Some("Integer") => {
            asm.invoke("virtual", key, &[0, 4]);
            2
        }
        Some(other) => {
            return Err(CompileError::bare(format!(
                "OnChange parameter {other} is not supported"
            )));
        }
    };
    asm.ret(0, "void");
    cls.method(MethodModel::new(
        "onValueChange",
        "V",
        vec![
            "Landroid/widget/NumberPicker;".into(),
            "I".into(),
            "I".into(),
        ],
        ACC_PUBLIC,
        Some(asm),
        5,
        4,
        outs,
    ));
    Ok(())
}

fn emit_time(
    cls: &mut ClassModel,
    desc: &str,
    form_desc: &str,
    rec: &RoutineInfo,
) -> Result<(), CompileError> {
    if !rec.params.is_empty() {
        return Err(CompileError::bare(format!(
            "{} takes no parameters",
            rec.routine.name
        )));
    }
    let mut asm = Asm::new();
    // registers 5, ins 4. v1 this, v2 picker, v3 hour, v4 minute, v0 form.
    asm.iget_obj(0, 1, field(desc, "this$0", form_desc));
    let key = handler_key(form_desc, rec)?;
    asm.invoke("virtual", key, &[0]);
    asm.ret(0, "void");
    cls.method(MethodModel::new(
        "onTimeChanged",
        "V",
        vec![
            "Landroid/widget/TimePicker;".into(),
            "I".into(),
            "I".into(),
        ],
        ACC_PUBLIC,
        Some(asm),
        5,
        4,
        1,
    ));
    Ok(())
}

fn emit_date_changed(
    cls: &mut ClassModel,
    desc: &str,
    form_desc: &str,
    rec: &RoutineInfo,
) -> Result<(), CompileError> {
    if !rec.params.is_empty() {
        return Err(CompileError::bare(format!(
            "{} takes no parameters",
            rec.routine.name
        )));
    }
    let mut asm = Asm::new();
    // registers 6, ins 5. v1 this, v2 picker, v3 year, v4 month, v5 day, v0 form.
    asm.iget_obj(0, 1, field(desc, "this$0", form_desc));
    let key = handler_key(form_desc, rec)?;
    asm.invoke("virtual", key, &[0]);
    asm.ret(0, "void");
    cls.method(MethodModel::new(
        "onDateChanged",
        "V",
        vec![
            "Landroid/widget/DatePicker;".into(),
            "I".into(),
            "I".into(),
            "I".into(),
        ],
        ACC_PUBLIC,
        Some(asm),
        6,
        5,
        1,
    ));
    Ok(())
}

fn emit_update_date(asm: &mut Asm, props: Option<&Map<String, Value>>) -> Result<(), CompileError> {
    // v3–v5 are free until LayoutParams. The widget month is 0–11.
    let defaults = default_props("Date");
    let year = prop_or(props, &defaults, "Year")?;
    let month = prop_or(props, &defaults, "Month")?;
    let day = prop_or(props, &defaults, "Day")?;
    asm.const_(3, year);
    asm.const_(4, month - 1);
    asm.const_(5, day);
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/DatePicker;",
            "updateDate",
            "V",
            &["I", "I", "I"],
        ),
        &[2, 3, 4, 5],
    );
    Ok(())
}

fn build_frame(
    is_static: bool,
    params: &[(String, String)],
    locals: &[(String, String)],
) -> Result<Frame, CompileError> {
    let mut frame = Frame {
        order: Vec::new(),
        slots: HashMap::new(),
        param_names: params.iter().map(|(name, _)| name.to_lowercase()).collect(),
        this: None,
        registers: 0,
        ins: 0,
    };
    let mut reg = SCRATCH;
    for (name, ty) in locals {
        frame.insert(name, reg, ty.clone());
        reg = next_reg(reg)?;
    }
    let local_end = reg;
    if !is_static {
        frame.this = Some(reg);
        reg = next_reg(reg)?;
    }
    for (name, ty) in params {
        frame.insert(name, reg, ty.clone());
        reg = next_reg(reg)?;
    }
    if reg > 16 {
        return Err(CompileError::bare(
            "routine is too large for the register window (max 16)",
        ));
    }
    frame.registers = reg;
    frame.ins = reg - local_end;
    Ok(frame)
}

impl Frame {
    fn insert(&mut self, name: &str, reg: u16, ty: String) {
        let key = name.to_lowercase();
        if !self.slots.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.slots.insert(key, (reg, ty));
    }

    fn get(&self, name: &str) -> Result<(u16, String), CompileError> {
        self.slots
            .get(name)
            .map(|(reg, ty)| (*reg, ty.clone()))
            .ok_or_else(|| CompileError::bare(format!("unknown name {name}")))
    }
}

fn next_reg(reg: u16) -> Result<u16, CompileError> {
    reg.checked_add(1)
        .ok_or_else(|| CompileError::bare("routine is too large for the register window (max 16)"))
}

fn by_index(routines: &std::collections::BTreeMap<String, RoutineInfo>) -> Vec<&RoutineInfo> {
    let mut list: Vec<&RoutineInfo> = routines.values().collect();
    list.sort_by_key(|rec| rec.index);
    list
}

fn ordered_widgets(form: &FormChecked) -> Vec<&WidgetRef> {
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for comp in components(&form.form) {
        let Some(name) = comp.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let key = name.to_lowercase();
        if seen.insert(key.clone()) {
            if let Some(widget) = form.widgets.get(&key) {
                order.push(widget);
            }
        }
    }
    order
}

fn result_uses(form: &FormChecked) -> Vec<ResultUse> {
    let mut uses = Vec::new();
    for rec in by_index(&form.routines) {
        collect_results(&rec.routine.body, &mut uses);
    }
    uses
}

fn collect_results(stmts: &[Stmt], uses: &mut Vec<ResultUse>) {
    for stmt in stmts {
        collect_result_stmt(stmt, uses);
    }
}

fn collect_result_stmt(stmt: &Stmt, uses: &mut Vec<ResultUse>) {
    match stmt {
        Stmt::Block(block) => collect_results(&block.stmts, uses),
        Stmt::If(stmt) => {
            collect_result_expr(&stmt.cond, uses);
            collect_result_stmt(&stmt.then, uses);
            collect_result_stmt(&stmt.else_, uses);
        }
        Stmt::While(stmt) => {
            collect_result_expr(&stmt.cond, uses);
            collect_result_stmt(&stmt.body, uses);
        }
        Stmt::Assign(stmt) => collect_result_expr(&stmt.expr, uses),
        Stmt::Call(stmt) => {
            note_result(stmt.resolved.as_ref(), stmt.form_name.as_deref(), uses);
            for arg in &stmt.args {
                collect_result_expr(arg, uses);
            }
        }
        Stmt::Exit(_) => {}
    }
}

fn collect_result_expr(expr: &Expr, uses: &mut Vec<ResultUse>) {
    match expr {
        Expr::Call(call) => {
            note_result(call.resolved.as_ref(), call.form_name.as_deref(), uses);
            for arg in &call.args {
                collect_result_expr(arg, uses);
            }
        }
        Expr::Unary(unary) => collect_result_expr(&unary.expr, uses),
        Expr::Binary(binary) => {
            collect_result_expr(&binary.left, uses);
            collect_result_expr(&binary.right, uses);
        }
        Expr::Ident(_) | Expr::Literal(_) | Expr::Designator(_) => {}
    }
}

fn note_result(resolved: Option<&Resolved>, form_name: Option<&str>, uses: &mut Vec<ResultUse>) {
    let Some(Resolved::Builtin(name)) = resolved else {
        return;
    };
    let photo = match name.as_str() {
        "TakePhoto" => true,
        "OpenDocument" => false,
        _ => return,
    };
    let Some(handler) = form_name.filter(|handler| !handler.is_empty()) else {
        return;
    };
    if uses.iter().any(|item| item.photo == photo && item.handler.eq_ignore_ascii_case(handler)) {
        return;
    }
    let code = if photo {
        1 + uses.iter().filter(|item| item.photo).count() as i64
    } else {
        256 + uses.iter().filter(|item| !item.photo).count() as i64
    };
    uses.push(ResultUse {
        photo,
        handler: handler.to_string(),
        code,
    });
}

fn ask_handlers(form: &FormChecked) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for rec in by_index(&form.routines) {
        collect_stmts(&rec.routine.body, &mut seen, &mut order);
    }
    order
}

fn collect_stmts(stmts: &[Stmt], seen: &mut BTreeSet<String>, order: &mut Vec<String>) {
    for stmt in stmts {
        collect_stmt(stmt, seen, order);
    }
}

fn collect_stmt(stmt: &Stmt, seen: &mut BTreeSet<String>, order: &mut Vec<String>) {
    match stmt {
        Stmt::Block(block) => collect_stmts(&block.stmts, seen, order),
        Stmt::If(stmt) => {
            collect_expr(&stmt.cond, seen, order);
            collect_stmt(&stmt.then, seen, order);
            collect_stmt(&stmt.else_, seen, order);
        }
        Stmt::While(stmt) => {
            collect_expr(&stmt.cond, seen, order);
            collect_stmt(&stmt.body, seen, order);
        }
        Stmt::Assign(stmt) => collect_expr(&stmt.expr, seen, order),
        Stmt::Call(stmt) => {
            note_question(
                stmt.resolved.as_ref(),
                stmt.form_name.as_deref(),
                seen,
                order,
            );
            for arg in &stmt.args {
                collect_expr(arg, seen, order);
            }
        }
        Stmt::Exit(_) => {}
    }
}

fn collect_expr(expr: &Expr, seen: &mut BTreeSet<String>, order: &mut Vec<String>) {
    match expr {
        Expr::Call(call) => {
            note_question(
                call.resolved.as_ref(),
                call.form_name.as_deref(),
                seen,
                order,
            );
            for arg in &call.args {
                collect_expr(arg, seen, order);
            }
        }
        Expr::Unary(unary) => collect_expr(&unary.expr, seen, order),
        Expr::Binary(binary) => {
            collect_expr(&binary.left, seen, order);
            collect_expr(&binary.right, seen, order);
        }
        Expr::Ident(_) | Expr::Literal(_) | Expr::Designator(_) => {}
    }
}

fn note_question(
    resolved: Option<&Resolved>,
    form_name: Option<&str>,
    seen: &mut BTreeSet<String>,
    order: &mut Vec<String>,
) {
    let Some(Resolved::Builtin(name)) = resolved else {
        return;
    };
    if name != "Question" {
        return;
    }
    let Some(handler) = form_name.filter(|handler| !handler.is_empty()) else {
        return;
    };
    if seen.insert(handler.to_string()) {
        order.push(handler.to_string());
    }
}

fn handler_key(desc: &str, rec: &RoutineInfo) -> Result<MethodRef, CompileError> {
    let ret = match &rec.ret {
        None => "V".to_string(),
        Some(ty) => type_desc(ty)?,
    };
    let params = rec
        .params
        .iter()
        .map(|ty| type_desc(ty))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MethodRef {
        owner: desc.to_string(),
        name: rec.routine.name.clone(),
        ret,
        params,
    })
}

fn paired_params(rec: &RoutineInfo) -> Result<Vec<(String, String)>, CompileError> {
    if rec.routine.params.len() != rec.params.len() {
        return Err(CompileError::bare("parameter mismatch"));
    }
    Ok(rec
        .routine
        .params
        .iter()
        .zip(rec.params.iter())
        .map(|(param, ty)| (param.name.clone(), ty.clone()))
        .collect())
}

fn flatten_strcat(expr: &Expr, parts: &mut Vec<Expr>) {
    if let Expr::Binary(bin) = expr {
        if bin.op == "+" && bin.string_add {
            flatten_strcat(&bin.left, parts);
            flatten_strcat(&bin.right, parts);
            return;
        }
    }
    parts.push(expr.clone());
}

fn expr_type(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Ident(node) => node.ty.as_deref(),
        Expr::Literal(node) => node.ty.as_deref().or(Some(node.type_name.as_str())),
        Expr::Unary(node) => node.ty.as_deref(),
        Expr::Binary(node) => node.ty.as_deref(),
        Expr::Call(node) => node.ty.as_deref(),
        Expr::Designator(node) => node.ty.as_deref(),
    }
}

fn extra_put(expr: &Expr) -> Result<(&'static str, &'static str), CompileError> {
    match expr_type(expr) {
        Some("String") => Ok(("S", "Ljava/lang/String;")),
        Some("Integer") => Ok(("I", "I")),
        Some("Boolean") => Ok(("B", "Z")),
        _ => Err(CompileError::bare("OpenForm cannot pass a View")),
    }
}

fn open_form_name(form_name: Option<&str>, args: &[Expr]) -> Result<String, CompileError> {
    if let Some(name) = form_name {
        if !name.is_empty() {
            return Ok(name.to_string());
        }
    }
    match args.first() {
        Some(Expr::Literal(lit)) => match &lit.value {
            Lit::Str(text) => Ok(text.clone()),
            _ => Err(CompileError::bare("OpenForm needs a form name")),
        },
        _ => Err(CompileError::bare("OpenForm needs a form name")),
    }
}

/// True for an optional sign and one to nine digits. `parseInt` accepts that
/// range, so a false result can skip `StrToInt` instead of killing the activity.
fn emit_is_int(asm: &mut Asm) {
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[4],
    );
    asm.move_result(1, false);
    asm.if_zero("eqz", 1, "is_fail");
    asm.const_(0, 0);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "charAt", "C", &["I"]),
        &[4, 0],
    );
    asm.move_result(2, false);
    asm.const_(3, 43);
    asm.if_cmp("eq", 2, 3, "is_signed");
    asm.const_(3, 45);
    asm.if_cmp("ne", 2, 3, "is_bounds");
    asm.label("is_signed");
    asm.const_(0, 1);
    asm.label("is_bounds");
    asm.if_cmp("ge", 0, 1, "is_fail");
    asm.const_(3, 9);
    asm.binop(0x90, 3, 3, 0, "add-int");
    asm.if_cmp("gt", 1, 3, "is_fail");
    asm.label("is_loop");
    asm.if_cmp("ge", 0, 1, "is_ok");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "charAt", "C", &["I"]),
        &[4, 0],
    );
    asm.move_result(2, false);
    asm.const_(3, 48);
    asm.if_cmp("lt", 2, 3, "is_fail");
    asm.const_(3, 57);
    asm.if_cmp("gt", 2, 3, "is_fail");
    asm.const_(3, 1);
    asm.binop(0x90, 0, 0, 3, "add-int");
    asm.goto("is_loop");
    asm.label("is_ok");
    asm.const_(0, 1);
    asm.ret(0, "int");
    asm.label("is_fail");
    asm.const_(0, 0);
    asm.ret(0, "int");
}

/// `v5` is the context, `v6` the name, `v7` the text. The file lives under the app's files dir.
fn emit_write_text(asm: &mut Asm) {
    asm.invoke(
        "virtual",
        method("Landroid/content/Context;", "getFilesDir", "Ljava/io/File;", &[]),
        &[5],
    );
    asm.move_result(0, true);
    asm.new_instance(1, "Ljava/io/File;");
    asm.invoke(
        "direct",
        method(
            "Ljava/io/File;",
            "<init>",
            "V",
            &["Ljava/io/File;", "Ljava/lang/String;"],
        ),
        &[1, 0, 6],
    );
    asm.new_instance(2, "Ljava/io/FileOutputStream;");
    asm.invoke(
        "direct",
        method(
            "Ljava/io/FileOutputStream;",
            "<init>",
            "V",
            &["Ljava/io/File;"],
        ),
        &[2, 1],
    );
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "getBytes", "[B", &[]),
        &[7],
    );
    asm.move_result(3, true);
    asm.invoke(
        "virtual",
        method("Ljava/io/FileOutputStream;", "write", "V", &["[B"]),
        &[2, 3],
    );
    asm.invoke(
        "virtual",
        method("Ljava/io/FileOutputStream;", "close", "V", &[]),
        &[2],
    );
    asm.ret(0, "void");
}

/// Missing and empty files both return "". The two returns stay apart so a scanner
/// register does not join a path that never created one.
fn emit_read_text(asm: &mut Asm) {
    asm.invoke(
        "virtual",
        method("Landroid/content/Context;", "getFilesDir", "Ljava/io/File;", &[]),
        &[6],
    );
    asm.move_result(0, true);
    asm.new_instance(1, "Ljava/io/File;");
    asm.invoke(
        "direct",
        method(
            "Ljava/io/File;",
            "<init>",
            "V",
            &["Ljava/io/File;", "Ljava/lang/String;"],
        ),
        &[1, 0, 7],
    );
    asm.invoke(
        "virtual",
        method("Ljava/io/File;", "exists", "Z", &[]),
        &[1],
    );
    asm.move_result(2, false);
    asm.if_zero("eqz", 2, "missing");
    asm.new_instance(2, "Ljava/io/FileInputStream;");
    asm.invoke(
        "direct",
        method(
            "Ljava/io/FileInputStream;",
            "<init>",
            "V",
            &["Ljava/io/File;"],
        ),
        &[2, 1],
    );
    asm.new_instance(3, "Ljava/util/Scanner;");
    asm.invoke(
        "direct",
        method(
            "Ljava/util/Scanner;",
            "<init>",
            "V",
            &["Ljava/io/InputStream;"],
        ),
        &[3, 2],
    );
    // `\A` matches the start of the input, so the next token is the whole file.
    asm.const_string(4, "\\A");
    asm.invoke(
        "virtual",
        method(
            "Ljava/util/Scanner;",
            "useDelimiter",
            "Ljava/util/Scanner;",
            &["Ljava/lang/String;"],
        ),
        &[3, 4],
    );
    asm.move_result(3, true);
    asm.invoke(
        "virtual",
        method("Ljava/util/Scanner;", "hasNext", "Z", &[]),
        &[3],
    );
    asm.move_result(4, false);
    asm.if_zero("eqz", 4, "empty");
    asm.invoke(
        "virtual",
        method("Ljava/util/Scanner;", "next", "Ljava/lang/String;", &[]),
        &[3],
    );
    asm.move_result(0, true);
    asm.invoke(
        "virtual",
        method("Ljava/util/Scanner;", "close", "V", &[]),
        &[3],
    );
    asm.ret(0, "obj");
    asm.label("empty");
    asm.invoke(
        "virtual",
        method("Ljava/util/Scanner;", "close", "V", &[]),
        &[3],
    );
    asm.const_string(0, "");
    asm.ret(0, "obj");
    asm.label("missing");
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

/// `v11` is the context. Doubles stay in this method: the dialect has no wide type.
fn emit_where(asm: &mut Asm) {
    asm.const_string(0, "android.permission.ACCESS_FINE_LOCATION");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "checkSelfPermission",
            "I",
            &["Ljava/lang/String;"],
        ),
        &[11, 0],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "granted");
    asm.const_(0, 2);
    asm.new_array(1, 0, "[Ljava/lang/String;");
    asm.const_string(2, "android.permission.ACCESS_FINE_LOCATION");
    asm.const_(0, 0);
    asm.aput_object(2, 1, 0);
    asm.const_string(2, "android.permission.ACCESS_COARSE_LOCATION");
    asm.const_(0, 1);
    asm.aput_object(2, 1, 0);
    asm.check_cast(11, "Landroid/app/Activity;");
    asm.const_(0, 1);
    asm.invoke(
        "virtual",
        method(
            "Landroid/app/Activity;",
            "requestPermissions",
            "V",
            &["[Ljava/lang/String;", "I"],
        ),
        &[11, 1, 0],
    );
    asm.const_string(0, "Allow location");
    asm.ret(0, "obj");
    asm.label("granted");
    asm.const_string(0, "location");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "getSystemService",
            "Ljava/lang/Object;",
            &["Ljava/lang/String;"],
        ),
        &[11, 0],
    );
    asm.move_result(0, true);
    asm.check_cast(0, "Landroid/location/LocationManager;");
    asm.const_string(1, "gps");
    asm.invoke(
        "virtual",
        method(
            "Landroid/location/LocationManager;",
            "getLastKnownLocation",
            "Landroid/location/Location;",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    asm.move_result(1, true);
    asm.if_zero("nez", 1, "have");
    asm.const_string(1, "network");
    asm.invoke(
        "virtual",
        method(
            "Landroid/location/LocationManager;",
            "getLastKnownLocation",
            "Landroid/location/Location;",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    asm.move_result(1, true);
    asm.if_zero("nez", 1, "have");
    asm.const_string(1, "passive");
    asm.invoke(
        "virtual",
        method(
            "Landroid/location/LocationManager;",
            "getLastKnownLocation",
            "Landroid/location/Location;",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    asm.move_result(1, true);
    asm.if_zero("eqz", 1, "none");
    asm.label("have");
    asm.invoke(
        "virtual",
        method("Landroid/location/Location;", "getLatitude", "D", &[]),
        &[1],
    );
    asm.move_result_wide(2);
    // A double is two register words, so valueOf's invoke count is 2.
    asm.invoke(
        "static",
        method("Ljava/lang/String;", "valueOf", "Ljava/lang/String;", &["D"]),
        &[2, 3],
    );
    asm.move_result(4, true);
    asm.invoke(
        "virtual",
        method("Landroid/location/Location;", "getLongitude", "D", &[]),
        &[1],
    );
    asm.move_result_wide(2);
    asm.invoke(
        "static",
        method("Ljava/lang/String;", "valueOf", "Ljava/lang/String;", &["D"]),
        &[2, 3],
    );
    asm.move_result(5, true);
    asm.const_string(6, ", ");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "concat",
            "Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[4, 6],
    );
    asm.move_result(4, true);
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "concat",
            "Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[4, 5],
    );
    asm.move_result(4, true);
    asm.ret(4, "obj");
    asm.label("none");
    asm.const_string(0, "No location");
    asm.ret(0, "obj");
}

/// `v5` is the context. This starts the camera and does not wait for a picture.
/// `TakePhoto` is the call that receives the thumbnail.
/// `v6` is the activity and `v7` is the 0-based extra index.
/// `v3` is the only integer compare, so `v1` stays a string or untouched at
/// `none`. An int and a reference do not merge, and every miss path joins there.
/// The value returns on its own instruction, so the intent in `v0` is not joined
/// with that int. A missing, wrong-type, or out-of-range extra is "", 0, or false.
fn emit_form_arg(asm: &mut Asm, kind: &str) {
    asm.invoke(
        "virtual",
        method(
            "Landroid/app/Activity;",
            "getIntent",
            "Landroid/content/Intent;",
            &[],
        ),
        &[6],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, "none");
    emit_arg_keys(asm, "t", "tk", "tgot");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Intent;",
            "getStringExtra",
            "Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, "none");
    asm.const_string(3, kind);
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "equals",
            "Z",
            &["Ljava/lang/Object;"],
        ),
        &[2, 3],
    );
    asm.move_result(3, false);
    asm.if_zero("eqz", 3, "none");
    emit_arg_keys(asm, "a", "ak", "agot");
    match kind {
        "S" => {
            asm.invoke(
                "virtual",
                method(
                    "Landroid/content/Intent;",
                    "getStringExtra",
                    "Ljava/lang/String;",
                    &["Ljava/lang/String;"],
                ),
                &[0, 1],
            );
            asm.move_result(2, true);
            asm.if_zero("eqz", 2, "none");
            asm.ret(2, "obj");
        }
        "I" => {
            asm.const_(3, 0);
            asm.invoke(
                "virtual",
                method(
                    "Landroid/content/Intent;",
                    "getIntExtra",
                    "I",
                    &["Ljava/lang/String;", "I"],
                ),
                &[0, 1, 3],
            );
            asm.move_result(2, false);
            asm.ret(2, "int");
        }
        "B" => {
            asm.const_(3, 0);
            asm.invoke(
                "virtual",
                method(
                    "Landroid/content/Intent;",
                    "getBooleanExtra",
                    "Z",
                    &["Ljava/lang/String;", "Z"],
                ),
                &[0, 1, 3],
            );
            asm.move_result(2, false);
            asm.ret(2, "int");
        }
        _ => unreachable!("form arg kind"),
    }
    asm.label("none");
    if kind == "S" {
        asm.const_string(2, "");
        asm.ret(2, "obj");
    } else {
        asm.const_(2, 0);
        asm.ret(2, "int");
    }
}

fn emit_arg_keys(asm: &mut Asm, prefix: &str, lab: &str, got: &str) {
    for index in 0..4 {
        asm.const_(3, index);
        asm.if_cmp("eq", 7, 3, &format!("{lab}{index}"));
    }
    asm.goto("none");
    for index in 0..4 {
        asm.label(&format!("{lab}{index}"));
        asm.const_string(1, &format!("{prefix}{index}"));
        if index != 3 {
            asm.goto(got);
        }
    }
    asm.label(got);
}

fn emit_open_camera(asm: &mut Asm) {
    asm.new_instance(0, "Landroid/content/Intent;");
    asm.const_string(1, "android.media.action.IMAGE_CAPTURE");
    asm.invoke(
        "direct",
        method(
            "Landroid/content/Intent;",
            "<init>",
            "V",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "getPackageManager",
            "Landroid/content/pm/PackageManager;",
            &[],
        ),
        &[5],
    );
    asm.move_result(2, true);
    asm.const_(3, 0);
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/pm/PackageManager;",
            "queryIntentActivities",
            "Ljava/util/List;",
            &["Landroid/content/Intent;", "I"],
        ),
        &[2, 0, 3],
    );
    asm.move_result(2, true);
    asm.invoke(
        "interface",
        method("Ljava/util/List;", "isEmpty", "Z", &[]),
        &[2],
    );
    asm.move_result(3, false);
    asm.if_zero("nez", 3, "done");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "startActivity",
            "V",
            &["Landroid/content/Intent;"],
        ),
        &[5, 0],
    );
    asm.label("done");
    asm.ret(0, "void");
}

/// `v6` is the activity and `v7` is the request code. An empty resolve list returns 0.
fn emit_take_photo(asm: &mut Asm) {
    asm.new_instance(0, "Landroid/content/Intent;");
    asm.const_string(1, "android.media.action.IMAGE_CAPTURE");
    asm.invoke(
        "direct",
        method(
            "Landroid/content/Intent;",
            "<init>",
            "V",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    emit_for_result(asm);
}

/// `v6` is the activity and `v7` is the request code. The chosen stream is copied later.
fn emit_open_document(asm: &mut Asm) {
    asm.new_instance(0, "Landroid/content/Intent;");
    asm.const_string(1, "android.intent.action.OPEN_DOCUMENT");
    asm.invoke(
        "direct",
        method(
            "Landroid/content/Intent;",
            "<init>",
            "V",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    asm.const_string(1, "android.intent.category.OPENABLE");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Intent;",
            "addCategory",
            "Landroid/content/Intent;",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.const_string(1, "*/*");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Intent;",
            "setType",
            "Landroid/content/Intent;",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    asm.move_result(0, true);
    emit_for_result(asm);
}

fn emit_for_result(asm: &mut Asm) {
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "getPackageManager",
            "Landroid/content/pm/PackageManager;",
            &[],
        ),
        &[6],
    );
    asm.move_result(2, true);
    asm.const_(3, 0);
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/pm/PackageManager;",
            "queryIntentActivities",
            "Ljava/util/List;",
            &["Landroid/content/Intent;", "I"],
        ),
        &[2, 0, 3],
    );
    asm.move_result(2, true);
    asm.invoke(
        "interface",
        method("Ljava/util/List;", "isEmpty", "Z", &[]),
        &[2],
    );
    asm.move_result(3, false);
    asm.if_zero("nez", 3, "miss");
    asm.invoke(
        "virtual",
        method(
            "Landroid/app/Activity;",
            "startActivityForResult",
            "V",
            &["Landroid/content/Intent;", "I"],
        ),
        &[6, 0, 7],
    );
    asm.const_(4, 1);
    asm.ret(4, "int");
    asm.label("miss");
    asm.const_(4, 0);
    asm.ret(4, "int");
}

/// `v6` is the context and `v7` is the result intent. The file is the camera
/// thumbnail, written over `droide-photo.jpg`. A missing bitmap returns "".
fn emit_photo_path(asm: &mut Asm) {
    asm.if_zero("eqz", 7, "none");
    asm.const_string(0, "data");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Intent;",
            "getParcelableExtra",
            "Landroid/os/Parcelable;",
            &["Ljava/lang/String;"],
        ),
        &[7, 0],
    );
    asm.move_result(1, true);
    asm.if_zero("eqz", 1, "none");
    asm.check_cast(1, "Landroid/graphics/Bitmap;");
    asm.invoke(
        "virtual",
        method("Landroid/content/Context;", "getFilesDir", "Ljava/io/File;", &[]),
        &[6],
    );
    asm.move_result(2, true);
    asm.new_instance(3, "Ljava/io/File;");
    asm.const_string(0, "droide-photo.jpg");
    asm.invoke(
        "direct",
        method(
            "Ljava/io/File;",
            "<init>",
            "V",
            &["Ljava/io/File;", "Ljava/lang/String;"],
        ),
        &[3, 2, 0],
    );
    asm.new_instance(4, "Ljava/io/FileOutputStream;");
    asm.invoke(
        "direct",
        method(
            "Ljava/io/FileOutputStream;",
            "<init>",
            "V",
            &["Ljava/io/File;"],
        ),
        &[4, 3],
    );
    asm.sget_object(
        0,
        field(
            "Landroid/graphics/Bitmap$CompressFormat;",
            "JPEG",
            "Landroid/graphics/Bitmap$CompressFormat;",
        ),
    );
    asm.const_(5, 90);
    asm.invoke(
        "virtual",
        method(
            "Landroid/graphics/Bitmap;",
            "compress",
            "Z",
            &[
                "Landroid/graphics/Bitmap$CompressFormat;",
                "I",
                "Ljava/io/OutputStream;",
            ],
        ),
        &[1, 0, 5, 4],
    );
    asm.move_result(5, false);
    asm.invoke(
        "virtual",
        method("Ljava/io/OutputStream;", "close", "V", &[]),
        &[4],
    );
    asm.invoke(
        "virtual",
        method("Ljava/io/File;", "getAbsolutePath", "Ljava/lang/String;", &[]),
        &[3],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("none");
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

/// `v10` is the context and `v11` is the result intent. The bytes are copied to
/// `droide-document` while the result grant is still valid. A cancel returns "".
fn emit_document_path(asm: &mut Asm) {
    asm.if_zero("eqz", 11, "none");
    asm.invoke(
        "virtual",
        method("Landroid/content/Intent;", "getData", "Landroid/net/Uri;", &[]),
        &[11],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, "none");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "getContentResolver",
            "Landroid/content/ContentResolver;",
            &[],
        ),
        &[10],
    );
    asm.move_result(1, true);
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/ContentResolver;",
            "openInputStream",
            "Ljava/io/InputStream;",
            &["Landroid/net/Uri;"],
        ),
        &[1, 0],
    );
    asm.move_result(2, true);
    asm.invoke(
        "virtual",
        method("Landroid/content/Context;", "getFilesDir", "Ljava/io/File;", &[]),
        &[10],
    );
    asm.move_result(3, true);
    asm.new_instance(4, "Ljava/io/File;");
    asm.const_string(5, "droide-document");
    asm.invoke(
        "direct",
        method(
            "Ljava/io/File;",
            "<init>",
            "V",
            &["Ljava/io/File;", "Ljava/lang/String;"],
        ),
        &[4, 3, 5],
    );
    asm.new_instance(6, "Ljava/io/FileOutputStream;");
    asm.invoke(
        "direct",
        method(
            "Ljava/io/FileOutputStream;",
            "<init>",
            "V",
            &["Ljava/io/File;"],
        ),
        &[6, 4],
    );
    asm.const_(7, 4096);
    asm.new_array(8, 7, "[B");
    asm.label("loop");
    asm.invoke(
        "virtual",
        method("Ljava/io/InputStream;", "read", "I", &["[B"]),
        &[2, 8],
    );
    asm.move_result(9, false);
    asm.const_(7, 0);
    asm.if_cmp("le", 9, 7, "copied");
    asm.invoke(
        "virtual",
        method("Ljava/io/OutputStream;", "write", "V", &["[B", "I", "I"]),
        &[6, 8, 7, 9],
    );
    asm.goto("loop");
    asm.label("copied");
    asm.invoke(
        "virtual",
        method("Ljava/io/InputStream;", "close", "V", &[]),
        &[2],
    );
    asm.invoke(
        "virtual",
        method("Ljava/io/OutputStream;", "close", "V", &[]),
        &[6],
    );
    asm.invoke(
        "virtual",
        method("Ljava/io/File;", "getAbsolutePath", "Ljava/lang/String;", &[]),
        &[4],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("none");
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

/// `v6` is the context and `v7` is the text. An empty handler list skips the chooser,
/// the same way the camera skips a capture intent nobody handles.
fn emit_share(asm: &mut Asm) {
    asm.new_instance(0, "Landroid/content/Intent;");
    asm.const_string(1, "android.intent.action.SEND");
    asm.invoke(
        "direct",
        method(
            "Landroid/content/Intent;",
            "<init>",
            "V",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    asm.const_string(1, "text/plain");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Intent;",
            "setType",
            "Landroid/content/Intent;",
            &["Ljava/lang/String;"],
        ),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.const_string(1, "android.intent.extra.TEXT");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Intent;",
            "putExtra",
            "Landroid/content/Intent;",
            &["Ljava/lang/String;", "Ljava/lang/String;"],
        ),
        &[0, 1, 7],
    );
    asm.move_result(0, true);
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "getPackageManager",
            "Landroid/content/pm/PackageManager;",
            &[],
        ),
        &[6],
    );
    asm.move_result(2, true);
    asm.const_(3, 0);
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/pm/PackageManager;",
            "queryIntentActivities",
            "Ljava/util/List;",
            &["Landroid/content/Intent;", "I"],
        ),
        &[2, 0, 3],
    );
    asm.move_result(2, true);
    asm.invoke(
        "interface",
        method("Ljava/util/List;", "isEmpty", "Z", &[]),
        &[2],
    );
    asm.move_result(3, false);
    asm.if_zero("nez", 3, "done");
    asm.const_string(1, "Share");
    asm.invoke(
        "static",
        method(
            "Landroid/content/Intent;",
            "createChooser",
            "Landroid/content/Intent;",
            &["Landroid/content/Intent;", "Ljava/lang/CharSequence;"],
        ),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "startActivity",
            "V",
            &["Landroid/content/Intent;"],
        ),
        &[6, 0],
    );
    asm.label("done");
    asm.ret(0, "void");
}

/// `v6` is the context and `v7` is the text. A missing clipboard service returns.
fn emit_copy_text(asm: &mut Asm) {
    asm.const_string(0, "clipboard");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "getSystemService",
            "Ljava/lang/Object;",
            &["Ljava/lang/String;"],
        ),
        &[6, 0],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, "done");
    asm.check_cast(0, "Landroid/content/ClipboardManager;");
    asm.const_string(1, "text");
    asm.invoke(
        "static",
        method(
            "Landroid/content/ClipData;",
            "newPlainText",
            "Landroid/content/ClipData;",
            &["Ljava/lang/CharSequence;", "Ljava/lang/CharSequence;"],
        ),
        &[1, 7],
    );
    asm.move_result(1, true);
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/ClipboardManager;",
            "setPrimaryClip",
            "V",
            &["Landroid/content/ClipData;"],
        ),
        &[0, 1],
    );
    asm.label("done");
    asm.ret(0, "void");
}

/// `v7` is the context. `v1` stays an int and `v2` stays a reference, so the
/// empty exits can share one label. `getText` is null for a URI or intent clip.
fn emit_line_count(asm: &mut Asm) {
    // v5 is the text.
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[5],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "empty");
    asm.const_string(1, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[5, 1],
    );
    asm.move_result(1, true);
    asm.array_length(0, 1);
    asm.ret(0, "int");
    asm.label("empty");
    asm.const_(0, 0);
    asm.ret(0, "int");
}

fn emit_toggle_line(asm: &mut Asm) {
    // 12 registers, ins 2. v10 is the text and v11 the line.
    // v0 stays an int at every label. v6 is the current part. v7 is a newline.
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[11],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "keep");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[10],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "only");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[10, 0],
    );
    asm.move_result(1, true);
    asm.array_length(2, 1);
    asm.new_instance(3, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        method("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[3],
    );
    asm.const_(4, 0);
    asm.const_(5, 0);
    asm.const_(0, 0);
    asm.const_string(6, "");
    asm.const_string(7, "");
    asm.label("loop");
    asm.if_cmp("ge", 4, 2, "finish");
    asm.aget_object(6, 1, 4);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "equals", "Z", &["Ljava/lang/Object;"]),
        &[6, 11],
    );
    asm.move_result(0, false);
    asm.if_zero("nez", 0, "drop");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[3],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "add");
    asm.const_string(7, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[3, 7],
    );
    asm.move_result(3, true);
    asm.const_(0, 0);
    asm.label("add");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[3, 6],
    );
    asm.move_result(3, true);
    asm.const_(0, 0);
    asm.goto("next");
    asm.label("drop");
    asm.const_(5, 1);
    asm.const_(0, 0);
    asm.label("next");
    asm.const_(0, 1);
    asm.binop(0x90, 4, 4, 0, "add-int");
    asm.const_(0, 0);
    asm.goto("loop");
    asm.label("finish");
    asm.if_zero("nez", 5, "built");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[3],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "just");
    asm.const_string(7, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[3, 7],
    );
    asm.move_result(3, true);
    asm.const_(0, 0);
    asm.label("just");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[3, 11],
    );
    asm.move_result(3, true);
    asm.label("built");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[3],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("only");
    asm.ret(11, "obj");
    asm.label("keep");
    asm.ret(10, "obj");
}

fn emit_drop_names(asm: &mut Asm) {
    // 14 registers, ins 2. v12 is the lines and v13 the names to remove.
    // v0 stays an int at every label. v8 is the current line. v9 is a name.
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[12],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "empty");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[12, 0],
    );
    asm.move_result(2, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[13],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "all");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[13, 0],
    );
    asm.move_result(1, true);
    asm.const_(0, 0);
    asm.goto("ready");
    asm.label("all");
    asm.const_(0, 0);
    asm.new_array(1, 0, "[Ljava/lang/String;");
    asm.label("ready");
    asm.array_length(3, 2);
    asm.array_length(7, 1);
    asm.new_instance(4, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        method("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[4],
    );
    asm.const_(5, 0);
    asm.const_(6, 0);
    asm.const_(0, 0);
    asm.const_string(8, "");
    asm.const_string(9, "");
    asm.label("loop");
    asm.if_cmp("ge", 5, 3, "done");
    asm.aget_object(8, 2, 5);
    asm.const_(6, 0);
    asm.label("scan");
    asm.if_cmp("ge", 6, 7, "keep");
    asm.aget_object(9, 1, 6);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "equals", "Z", &["Ljava/lang/Object;"]),
        &[8, 9],
    );
    asm.move_result(0, false);
    asm.if_zero("nez", 0, "skip");
    asm.const_(0, 1);
    asm.binop(0x90, 6, 6, 0, "add-int");
    asm.const_(0, 0);
    asm.goto("scan");
    asm.label("keep");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[4],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "add");
    asm.const_string(9, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[4, 9],
    );
    asm.move_result(4, true);
    asm.const_(0, 0);
    asm.label("add");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[4, 8],
    );
    asm.move_result(4, true);
    asm.const_(0, 0);
    asm.label("skip");
    asm.const_(0, 1);
    asm.binop(0x90, 5, 5, 0, "add-int");
    asm.const_(0, 0);
    asm.goto("loop");
    asm.label("done");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[4],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("empty");
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

fn emit_parallel(asm: &mut Asm, take: bool) {
    // v9 source lines, v10 key lines, v11 names. take keeps matches; otherwise drops them.
    let _ = take;
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

fn emit_join_notes(asm: &mut Asm, owner: &str) {
    // v10 context, v11 names. Reads note-<name> for each selected line.
    let _ = owner;
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

/// `v8` is the text and `v9` the day. One copy of the day for each line.
fn emit_same_days(asm: &mut Asm) {
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[8],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "empty");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[8, 0],
    );
    asm.move_result(2, true);
    asm.array_length(3, 2);
    asm.new_instance(4, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        method("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[4],
    );
    asm.const_(5, 0);
    asm.const_(0, 0);
    asm.const_(6, 0);
    asm.label("loop");
    asm.if_cmp("ge", 5, 3, "done");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[4],
    );
    asm.move_result(6, false);
    asm.if_zero("eqz", 6, "note");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[4, 0],
    );
    asm.move_result(4, true);
    asm.const_(0, 0);
    asm.label("note");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[4, 9],
    );
    asm.move_result(4, true);
    asm.const_(0, 0);
    asm.const_(6, 1);
    asm.binop(0x90, 5, 5, 6, "add-int");
    asm.const_(6, 0);
    asm.goto("loop");
    asm.label("done");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[4],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("empty");
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

/// `v10` titles, `v11` days. Inserts a day heading whenever the day changes.
fn append_line(asm: &mut Asm, text: u16, tag: &str) {
    let put = format!("ap{tag}");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[4],
    );
    asm.move_result(8, false);
    asm.if_zero("eqz", 8, &put);
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[4, 0],
    );
    asm.move_result(4, true);
    asm.const_string(0, "");
    asm.label(&put);
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[4, text],
    );
    asm.move_result(4, true);
    asm.const_(8, 0);
}

/// Append `current` when it differs from `prev`, then remember it.
/// In the details list the appended word is `word` (`Year`, `Month`, or `Day`).
fn emit_changed(asm: &mut Asm, current: u16, prev: u16, word: &str, details: bool, tag: &str) {
    let skip = format!("sk{tag}");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[current],
    );
    asm.move_result(8, false);
    asm.if_zero("eqz", 8, &skip);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "equals", "Z", &["Ljava/lang/Object;"]),
        &[current, prev],
    );
    asm.move_result(8, false);
    asm.if_zero("nez", 8, &skip);
    if details {
        asm.move_(prev, current, true);
        asm.const_string(current, word);
        append_line(asm, current, tag);
    } else {
        append_line(asm, current, tag);
        asm.move_(prev, current, true);
    }
    asm.const_(8, 0);
    asm.label(&skip);
    asm.const_(8, 0);
}

/// `v9` year, `v10` month name, `v11` day. English, matching `Today`.
fn emit_day_heading(asm: &mut Asm, rt: &str) {
    let months = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let weekdays = [
        "",
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    asm.invoke(
        "static",
        method(rt, "isInt", "Z", &["Ljava/lang/String;"]),
        &[9],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "plain_year");
    asm.invoke(
        "static",
        method("Ljava/lang/Integer;", "parseInt", "I", &["Ljava/lang/String;"]),
        &[9],
    );
    asm.move_result(1, false);
    asm.invoke(
        "static",
        method(rt, "isInt", "Z", &["Ljava/lang/String;"]),
        &[11],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "plain_day");
    asm.invoke(
        "static",
        method("Ljava/lang/Integer;", "parseInt", "I", &["Ljava/lang/String;"]),
        &[11],
    );
    asm.move_result(3, false);
    for (index, name) in months.iter().enumerate() {
        asm.const_string(0, name);
        asm.invoke(
            "virtual",
            method(
                "Ljava/lang/String;",
                "equalsIgnoreCase",
                "Z",
                &["Ljava/lang/String;"],
            ),
            &[10, 0],
        );
        asm.move_result(2, false);
        asm.if_zero("nez", 2, &format!("month{index}"));
    }
    asm.goto("plain_month");
    for index in 0..months.len() {
        asm.label(&format!("month{index}"));
        asm.const_(2, index as i64);
        asm.goto("dated");
    }
    asm.label("dated");
    asm.invoke(
        "static",
        method("Ljava/util/Calendar;", "getInstance", "Ljava/util/Calendar;", &[]),
        &[],
    );
    asm.move_result(4, true);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "clear", "V", &[]),
        &[4],
    );
    asm.const_(0, 1);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "set", "V", &["I", "I"]),
        &[4, 0, 1],
    );
    asm.const_(0, 2);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "set", "V", &["I", "I"]),
        &[4, 0, 2],
    );
    asm.const_(0, 5);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "set", "V", &["I", "I"]),
        &[4, 0, 3],
    );
    asm.const_(0, 7);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[4, 0],
    );
    asm.move_result(5, false);
    for index in 1..weekdays.len() {
        asm.const_(0, index as i64);
        asm.if_cmp("eq", 5, 0, &format!("week{index}"));
    }
    asm.goto("plain_dow");
    for (index, name) in weekdays.iter().enumerate().skip(1) {
        asm.label(&format!("week{index}"));
        asm.const_string(6, name);
        asm.goto("named");
    }
    asm.label("named");
    asm.const_string(0, ", ");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[6, 0],
    );
    asm.move_result(6, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[6, 10],
    );
    asm.move_result(6, true);
    asm.const_string(0, " ");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[6, 0],
    );
    asm.move_result(6, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[6, 11],
    );
    asm.move_result(6, true);
    asm.ret(6, "obj");
    for label in ["plain_year", "plain_day", "plain_month", "plain_dow"] {
        asm.label(label);
        asm.const_string(0, " ");
        asm.invoke(
            "virtual",
            method(
                "Ljava/lang/String;",
                "concat",
                "Ljava/lang/String;",
                &["Ljava/lang/String;"],
            ),
            &[10, 0],
        );
        asm.move_result(6, true);
        asm.invoke(
            "virtual",
            method(
                "Ljava/lang/String;",
                "concat",
                "Ljava/lang/String;",
                &["Ljava/lang/String;"],
            ),
            &[6, 11],
        );
        asm.move_result(6, true);
        asm.ret(6, "obj");
    }
}

/// `const/4 0` is the type zero, which will not join an int. Subtract to get a real int.
fn int_zero(asm: &mut Asm, dest: u16) {
    asm.const_(dest, 1);
    asm.binop(0x91, dest, dest, dest, "sub-int");
}

fn emit_insert_line(asm: &mut Asm) {
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[7],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "ins_only");
    int_zero(asm, 0);
    asm.if_cmp("ne", 9, 0, "ins_split");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[8, 0],
    );
    asm.move_result(0, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[0, 7],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("ins_split");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "split", "[Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[7, 0],
    );
    asm.move_result(0, true);
    asm.array_length(1, 0);
    asm.new_instance(2, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        method("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[2],
    );
    int_zero(asm, 3);
    int_zero(asm, 4);
    asm.const_string(5, "");
    int_zero(asm, 6);
    asm.label("ins_loop");
    int_zero(asm, 6);
    asm.if_cmp("ne", 3, 9, "ins_body");
    asm.if_cmp("ne", 4, 6, "ins_body");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[2],
    );
    asm.move_result(6, false);
    asm.if_zero("eqz", 6, "ins_mid");
    // v6 stays the length. A newline in that register will not join the empty path.
    asm.const_string(5, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[2, 5],
    );
    asm.move_result(2, true);
    asm.label("ins_mid");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[2, 8],
    );
    asm.move_result(2, true);
    asm.const_(4, 1);
    asm.label("ins_body");
    asm.if_cmp("ge", 3, 1, "ins_tail");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[2],
    );
    asm.move_result(6, false);
    asm.if_zero("eqz", 6, "ins_plain");
    asm.const_string(5, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[2, 5],
    );
    asm.move_result(2, true);
    asm.label("ins_plain");
    asm.aget_object(5, 0, 3);
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[2, 5],
    );
    asm.move_result(2, true);
    asm.const_(6, 1);
    asm.binop(0x90, 3, 3, 6, "add-int");
    asm.goto("ins_loop");
    asm.label("ins_tail");
    int_zero(asm, 6);
    asm.if_cmp("ne", 4, 6, "ins_out");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[2],
    );
    asm.move_result(6, false);
    asm.if_zero("eqz", 6, "ins_last");
    asm.const_string(5, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[2, 5],
    );
    asm.move_result(2, true);
    asm.label("ins_last");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[2, 8],
    );
    asm.move_result(2, true);
    asm.label("ins_out");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[2],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("ins_only");
    asm.ret(8, "obj");
}

fn emit_age_day(asm: &mut Asm) {
    let months = [
        "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December",
    ];
    asm.invoke(
        "static",
        method("Ljava/util/Calendar;", "getInstance", "Ljava/util/Calendar;", &[]),
        &[],
    );
    asm.move_result(0, true);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "clear", "V", &[]),
        &[0],
    );
    asm.const_(2, 1);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "set", "V", &["I", "I"]),
        &[0, 2, 7],
    );
    asm.const_(2, 2);
    asm.const_(3, 1);
    asm.binop(0x91, 3, 8, 3, "sub-int");
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "set", "V", &["I", "I"]),
        &[0, 2, 3],
    );
    asm.const_(2, 5);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "set", "V", &["I", "I"]),
        &[0, 2, 9],
    );
    asm.const_(2, 1);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[0, 2],
    );
    asm.move_result(4, false);
    asm.if_cmp("ne", 4, 7, "age_bad");
    asm.const_(2, 2);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[0, 2],
    );
    asm.move_result(4, false);
    asm.const_(3, 1);
    asm.binop(0x91, 3, 8, 3, "sub-int");
    asm.if_cmp("ne", 4, 3, "age_bad");
    asm.const_(2, 5);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[0, 2],
    );
    asm.move_result(4, false);
    asm.if_cmp("ne", 4, 9, "age_bad");
    asm.const_(2, 1);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "add", "V", &["I", "I"]),
        &[0, 2, 10],
    );
    asm.const_(2, 2);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[0, 2],
    );
    asm.move_result(4, false);
    asm.const_(3, 1);
    asm.binop(0x91, 3, 8, 3, "sub-int");
    asm.if_cmp("eq", 4, 3, "age_landed");
    asm.const_(2, 2);
    asm.const_(3, 1);
    asm.binop(0x90, 3, 8, 3, "add-int");
    asm.const_(4, 1);
    asm.binop(0x91, 3, 3, 4, "sub-int");
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "set", "V", &["I", "I"]),
        &[0, 2, 3],
    );
    asm.const_(2, 5);
    asm.const_(3, 1);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "set", "V", &["I", "I"]),
        &[0, 2, 3],
    );
    asm.const_(2, 5);
    asm.const_(3, -1);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "add", "V", &["I", "I"]),
        &[0, 2, 3],
    );
    asm.label("age_landed");
    int_zero(asm, 2);
    asm.if_cmp("eq", 11, 2, "age_begin");
    asm.const_(2, 1);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "add", "V", &["I", "I"]),
        &[0, 2, 2],
    );
    asm.label("age_begin");
    asm.invoke(
        "static",
        method("Ljava/util/Calendar;", "getInstance", "Ljava/util/Calendar;", &[]),
        &[],
    );
    asm.move_result(1, true);
    asm.const_(2, 1);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[0, 2],
    );
    asm.move_result(3, false);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[1, 2],
    );
    asm.move_result(4, false);
    asm.if_cmp("gt", 3, 4, "age_none");
    asm.if_cmp("lt", 3, 4, "age_keep");
    asm.const_(2, 2);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[0, 2],
    );
    asm.move_result(3, false);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[1, 2],
    );
    asm.move_result(4, false);
    asm.if_cmp("gt", 3, 4, "age_none");
    asm.if_cmp("lt", 3, 4, "age_keep");
    asm.const_(2, 5);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[0, 2],
    );
    asm.move_result(3, false);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[1, 2],
    );
    asm.move_result(4, false);
    asm.if_cmp("gt", 3, 4, "age_none");
    asm.label("age_keep");
    asm.const_(2, 0);
    asm.if_cmp("eq", 11, 2, "age_show");
    asm.const_(2, 5);
    asm.const_(3, -1);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "add", "V", &["I", "I"]),
        &[0, 2, 3],
    );
    asm.label("age_show");
    asm.new_instance(1, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        method("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[1],
    );
    asm.const_(2, 1);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[0, 2],
    );
    asm.move_result(3, false);
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["I"],
        ),
        &[1, 3],
    );
    asm.move_result(1, true);
    asm.const_string(3, " ");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[1, 3],
    );
    asm.move_result(1, true);
    asm.const_(2, 2);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[0, 2],
    );
    asm.move_result(4, false);
    for (index, name) in months.iter().enumerate() {
        asm.const_(3, index as i64);
        asm.if_cmp("eq", 4, 3, &format!("age_mo{index}"));
    }
    asm.const_string(3, "January");
    asm.goto("age_named");
    for (index, name) in months.iter().enumerate() {
        asm.label(&format!("age_mo{index}"));
        asm.const_string(3, name);
        asm.goto("age_named");
    }
    asm.label("age_named");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[1, 3],
    );
    asm.move_result(1, true);
    asm.const_string(3, " ");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[1, 3],
    );
    asm.move_result(1, true);
    asm.const_(2, 5);
    asm.invoke(
        "virtual",
        method("Ljava/util/Calendar;", "get", "I", &["I"]),
        &[0, 2],
    );
    asm.move_result(3, false);
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["I"],
        ),
        &[1, 3],
    );
    asm.move_result(1, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[1],
    );
    asm.move_result(1, true);
    asm.ret(1, "obj");
    asm.label("age_none");
    asm.const_string(0, "");
    asm.ret(0, "obj");
    asm.label("age_bad");
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

fn emit_clear_ages(asm: &mut Asm, rt: &str) {
    asm.const_string(0, "titles");
    asm.invoke(
        "static",
        method(rt, "readText", "Ljava/lang/String;", &["Landroid/content/Context;", "Ljava/lang/String;"]),
        &[11, 0],
    );
    asm.move_result(0, true);
    asm.const_string(1, "\n");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "split", "[Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.array_length(2, 0);
    asm.new_instance(3, "Ljava/lang/StringBuilder;");
    asm.invoke("direct", method("Ljava/lang/StringBuilder;", "<init>", "V", &[]), &[3]);
    int_zero(asm, 4);
    asm.const_string(5, "");
    asm.const_string(1, "");
    int_zero(asm, 6);
    asm.label("clr_loop");
    asm.if_cmp("ge", 4, 2, "clr_flags");
    asm.aget_object(5, 0, 4);
    asm.const_string(1, "Age ");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "startsWith", "Z", &["Ljava/lang/String;"]),
        &[5, 1],
    );
    asm.move_result(6, false);
    asm.if_zero("eqz", 6, "clr_yes");
    asm.const_(1, 4);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I"]),
        &[5, 1],
    );
    asm.move_result(1, true);
    asm.invoke(
        "static",
        method(rt, "isInt", "Z", &["Ljava/lang/String;"]),
        &[1],
    );
    asm.move_result(6, false);
    asm.if_zero("nez", 6, "clr_no");
    asm.label("clr_yes");
    asm.const_string(1, "1");
    asm.goto("clr_bit");
    asm.label("clr_no");
    asm.const_string(1, "0");
    asm.label("clr_bit");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "append", "Ljava/lang/StringBuilder;", &["Ljava/lang/String;"]),
        &[3, 1],
    );
    asm.move_result(3, true);
    asm.const_(6, 1);
    asm.binop(0x90, 4, 4, 6, "add-int");
    asm.const_string(5, "");
    asm.const_string(1, "");
    int_zero(asm, 6);
    asm.goto("clr_loop");
    asm.label("clr_flags");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[3],
    );
    asm.move_result(3, true);
    for name in ["titles", "days", "kinds", "marks"] {
        asm.const_string(0, name);
        asm.invoke(
            "static",
            method(rt, "readText", "Ljava/lang/String;", &["Landroid/content/Context;", "Ljava/lang/String;"]),
            &[11, 0],
        );
        asm.move_result(1, true);
        asm.invoke(
            "static",
            method(rt, "filterLines", "Ljava/lang/String;", &["Ljava/lang/String;", "Ljava/lang/String;"]),
            &[1, 3],
        );
        asm.move_result(1, true);
        asm.const_string(0, name);
        asm.invoke(
            "static",
            method(rt, "writeText", "V", &["Landroid/content/Context;", "Ljava/lang/String;", "Ljava/lang/String;"]),
            &[11, 0, 1],
        );
    }
    asm.ret(0, "void");
}

/// `v8` text, `v9` flags of 1 keep and 0 drop. Drops a line when its flag is 0.
fn emit_filter_lines(asm: &mut Asm) {
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "split", "[Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[8, 0],
    );
    asm.move_result(0, true);
    asm.array_length(1, 0);
    asm.new_instance(2, "Ljava/lang/StringBuilder;");
    asm.invoke("direct", method("Ljava/lang/StringBuilder;", "<init>", "V", &[]), &[2]);
    int_zero(asm, 3);
    asm.const_string(4, "");
    asm.label("flt_loop");
    asm.if_cmp("ge", 3, 1, "flt_out");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[9],
    );
    asm.move_result(5, false);
    asm.if_cmp("ge", 3, 5, "flt_keep");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "charAt", "C", &["I"]),
        &[9, 3],
    );
    asm.move_result(5, false);
    asm.const_(6, 48);
    asm.if_cmp("eq", 5, 6, "flt_next");
    asm.label("flt_keep");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[2],
    );
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "flt_word");
    asm.const_string(5, "\n");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "append", "Ljava/lang/StringBuilder;", &["Ljava/lang/String;"]),
        &[2, 5],
    );
    asm.move_result(2, true);
    asm.label("flt_word");
    asm.aget_object(4, 0, 3);
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "append", "Ljava/lang/StringBuilder;", &["Ljava/lang/String;"]),
        &[2, 4],
    );
    asm.move_result(2, true);
    asm.const_string(4, "");
    asm.label("flt_next");
    asm.const_(5, 1);
    asm.binop(0x90, 3, 3, 5, "add-int");
    asm.goto("flt_loop");
    asm.label("flt_out");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[2],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

fn emit_place_age(asm: &mut Asm, rt: &str) {
    asm.const_string(0, "days");
    asm.invoke(
        "static",
        method(rt, "readText", "Ljava/lang/String;", &["Landroid/content/Context;", "Ljava/lang/String;"]),
        &[12, 0],
    );
    asm.move_result(0, true);
    asm.const_string(1, "\n");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "split", "[Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[0, 1],
    );
    asm.move_result(1, true);
    asm.array_length(2, 1);
    asm.invoke(
        "static",
        method(rt, "dateKey", "I", &["Ljava/lang/String;"]),
        &[14],
    );
    asm.move_result(3, false);
    asm.const_string(0, "newest");
    asm.invoke(
        "static",
        method(rt, "readText", "Ljava/lang/String;", &["Landroid/content/Context;", "Ljava/lang/String;"]),
        &[12, 0],
    );
    asm.move_result(0, true);
    asm.const_string(4, "1");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "equals", "Z", &["Ljava/lang/Object;"]),
        &[0, 4],
    );
    asm.move_result(4, false);
    int_zero(asm, 5);
    int_zero(asm, 0);
    int_zero(asm, 6);
    asm.label("age_at");
    asm.if_cmp("ge", 5, 2, "age_put");
    asm.aget_object(0, 1, 5);
    asm.invoke(
        "static",
        method(rt, "dateKey", "I", &["Ljava/lang/String;"]),
        &[0],
    );
    asm.move_result(6, false);
    int_zero(asm, 0);
    asm.if_cmp("eq", 6, 0, "age_skip");
    int_zero(asm, 0);
    asm.if_cmp("eq", 4, 0, "age_old");
    asm.if_cmp("lt", 6, 3, "age_put");
    asm.goto("age_skip");
    asm.label("age_old");
    asm.if_cmp("gt", 6, 3, "age_put");
    asm.label("age_skip");
    asm.const_(0, 1);
    asm.binop(0x90, 5, 5, 0, "add-int");
    int_zero(asm, 0);
    int_zero(asm, 6);
    asm.goto("age_at");
    asm.label("age_put");
    // index is v5. Insert title, begin day, Period, and a Period mark.
    asm.const_string(0, "titles");
    emit_place_file(asm, rt, 0, 13, 5);
    asm.const_string(0, "days");
    emit_place_file(asm, rt, 0, 14, 5);
    asm.const_string(0, "kinds");
    asm.const_string(6, "Period");
    emit_place_file(asm, rt, 0, 6, 5);
    asm.const_string(0, "marks");
    asm.const_string(6, "Period");
    emit_place_file(asm, rt, 0, 6, 5);
    asm.const_string(0, "note-");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[0, 13],
    );
    asm.move_result(0, true);
    asm.invoke(
        "static",
        method(rt, "writeText", "V", &["Landroid/content/Context;", "Ljava/lang/String;", "Ljava/lang/String;"]),
        &[12, 0, 13],
    );
    asm.const_string(0, "kind-");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[0, 13],
    );
    asm.move_result(0, true);
    asm.const_string(6, "Period");
    asm.invoke(
        "static",
        method(rt, "writeText", "V", &["Landroid/content/Context;", "Ljava/lang/String;", "Ljava/lang/String;"]),
        &[12, 0, 6],
    );
    asm.const_string(0, "has-");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[0, 13],
    );
    asm.move_result(0, true);
    asm.const_string(6, "1");
    asm.invoke(
        "static",
        method(rt, "writeText", "V", &["Landroid/content/Context;", "Ljava/lang/String;", "Ljava/lang/String;"]),
        &[12, 0, 6],
    );
    asm.const_string(0, "when-");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[0, 13],
    );
    asm.move_result(0, true);
    asm.invoke(
        "static",
        method(rt, "writeText", "V", &["Landroid/content/Context;", "Ljava/lang/String;", "Ljava/lang/String;"]),
        &[12, 0, 14],
    );
    asm.const_string(0, "end-");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[0, 13],
    );
    asm.move_result(0, true);
    asm.invoke(
        "static",
        method(rt, "writeText", "V", &["Landroid/content/Context;", "Ljava/lang/String;", "Ljava/lang/String;"]),
        &[12, 0, 15],
    );
    asm.ret(0, "void");
}

fn emit_place_file(asm: &mut Asm, rt: &str, name: u16, line: u16, index: u16) {
    asm.invoke(
        "static",
        method(rt, "readText", "Ljava/lang/String;", &["Landroid/content/Context;", "Ljava/lang/String;"]),
        &[12, name],
    );
    asm.move_result(7, true);
    asm.invoke(
        "static",
        method(rt, "insertLine", "Ljava/lang/String;", &["Ljava/lang/String;", "Ljava/lang/String;", "I"]),
        &[7, line, index],
    );
    asm.move_result(7, true);
    asm.invoke(
        "static",
        method(rt, "writeText", "V", &["Landroid/content/Context;", "Ljava/lang/String;", "Ljava/lang/String;"]),
        &[12, name, 7],
    );
}

fn emit_date_key(asm: &mut Asm) {
    let months = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[7],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "key_z0");
    asm.const_(1, 32);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "indexOf", "I", &["I"]),
        &[7, 1],
    );
    asm.move_result(1, false);
    asm.const_(0, 0);
    asm.if_cmp("lt", 1, 0, "key_z1");
    asm.const_(0, 0);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I", "I"]),
        &[7, 0, 1],
    );
    asm.move_result(2, true);
    asm.const_(0, 1);
    asm.binop(0x90, 1, 1, 0, "add-int");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I"]),
        &[7, 1],
    );
    asm.move_result(5, true);
    asm.const_(1, 32);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "indexOf", "I", &["I"]),
        &[5, 1],
    );
    asm.move_result(1, false);
    asm.const_(0, 0);
    asm.if_cmp("lt", 1, 0, "key_z2");
    asm.invoke(
        "static",
        method("Ljava/lang/Integer;", "parseInt", "I", &["Ljava/lang/String;"]),
        &[2],
    );
    asm.move_result(2, false);
    asm.const_(0, 0);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I", "I"]),
        &[5, 0, 1],
    );
    asm.move_result(6, true);
    asm.const_(0, 1);
    asm.binop(0x90, 1, 1, 0, "add-int");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I"]),
        &[5, 1],
    );
    asm.move_result(5, true);
    asm.invoke(
        "static",
        method("Ljava/lang/Integer;", "parseInt", "I", &["Ljava/lang/String;"]),
        &[5],
    );
    asm.move_result(4, false);
    for (index, name) in months.iter().enumerate() {
        asm.const_string(0, name);
        asm.invoke(
            "virtual",
            method(
                "Ljava/lang/String;",
                "equalsIgnoreCase",
                "Z",
                &["Ljava/lang/String;"],
            ),
            &[6, 0],
        );
        asm.move_result(3, false);
        asm.if_zero("nez", 3, &format!("key_m{index}"));
    }
    asm.goto("key_z3");
    for index in 0..months.len() {
        asm.label(&format!("key_m{index}"));
        asm.const_(3, (index as i64) + 1);
        asm.goto("key_math");
    }
    asm.label("key_math");
    asm.const_(0, 10_000);
    asm.binop(0x92, 2, 2, 0, "mul-int");
    asm.const_(0, 100);
    asm.binop(0x92, 0, 3, 0, "mul-int");
    asm.binop(0x90, 2, 2, 0, "add-int");
    asm.binop(0x90, 2, 2, 4, "add-int");
    asm.ret(2, "int");
    for label in ["key_z0", "key_z1", "key_z2", "key_z3"] {
        asm.label(label);
        asm.const_(0, 0);
        asm.ret(0, "int");
    }
}

fn emit_date_part(asm: &mut Asm, rt: &str) {
    asm.invoke(
        "static",
        method(rt, "dateKey", "I", &["Ljava/lang/String;"]),
        &[4],
    );
    asm.move_result(0, false);
    asm.const_(1, 0);
    asm.if_cmp("eq", 5, 1, "part_year");
    asm.const_(1, 1);
    asm.if_cmp("eq", 5, 1, "part_month");
    asm.const_(1, 100);
    asm.binop(0x94, 0, 0, 1, "rem-int");
    asm.ret(0, "int");
    asm.label("part_year");
    asm.const_(1, 10_000);
    asm.binop(0x93, 0, 0, 1, "div-int");
    asm.ret(0, "int");
    asm.label("part_month");
    asm.const_(1, 100);
    asm.binop(0x93, 0, 0, 1, "div-int");
    asm.const_(1, 100);
    asm.binop(0x94, 0, 0, 1, "rem-int");
    asm.ret(0, "int");
}

fn emit_weave(asm: &mut Asm, rt: &str) {
    // v11 which, v12 titles, v13 days, v14 marks, v15 ends.
    let split = |asm: &mut Asm, src: u16, dest: u16, tag: &str| {
        asm.invoke(
            "virtual",
            method("Ljava/lang/String;", "length", "I", &[]),
            &[src],
        );
        asm.move_result(8, false);
        asm.if_zero("eqz", 8, &format!("we_{tag}"));
        asm.const_string(8, "\n");
        asm.invoke(
            "virtual",
            method(
                "Ljava/lang/String;",
                "split",
                "[Ljava/lang/String;",
                &["Ljava/lang/String;"],
            ),
            &[src, 8],
        );
        asm.move_result(dest, true);
        asm.const_(8, 0);
        asm.goto(&format!("wd_{tag}"));
        asm.label(&format!("we_{tag}"));
        asm.const_(8, 0);
        asm.new_array(dest, 8, "[Ljava/lang/String;");
        asm.label(&format!("wd_{tag}"));
    };
    split(asm, 12, 0, "t");
    split(asm, 13, 1, "d");
    split(asm, 14, 2, "m");
    split(asm, 15, 3, "e");
    int_zero(asm, 12);
    int_zero(asm, 13);
    int_zero(asm, 14);
    int_zero(asm, 15);
    asm.array_length(8, 0);
    asm.new_array(4, 8, "[Ljava/lang/Integer;");
    asm.new_array(5, 8, "[Ljava/lang/Integer;");
    asm.new_array(6, 8, "[Ljava/lang/Integer;");
    int_zero(asm, 9);
    int_zero(asm, 10);
    asm.invoke(
        "static",
        method("Ljava/lang/Integer;", "valueOf", "Ljava/lang/Integer;", &["I"]),
        &[10],
    );
    asm.move_result(14, true);
    asm.label("weave_fill");
    asm.if_cmp("ge", 9, 8, "weave_filled");
    asm.array_length(10, 1);
    asm.if_cmp("ge", 9, 10, "weave_noday");
    asm.aget_object(10, 1, 9);
    asm.goto("weave_daykey");
    asm.label("weave_noday");
    asm.const_string(10, "");
    asm.label("weave_daykey");
    asm.invoke(
        "static",
        method(rt, "dateKey", "I", &["Ljava/lang/String;"]),
        &[10],
    );
    asm.move_result(10, false);
    int_zero(asm, 14);
    asm.if_cmp("ne", 10, 14, "weave_keepday");
    asm.if_cmp("eq", 9, 14, "weave_keepday");
    asm.const_(14, 1);
    asm.binop(0x91, 14, 9, 14, "sub-int");
    asm.aget_object(10, 4, 14);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Integer;", "intValue", "I", &[]),
        &[10],
    );
    asm.move_result(10, false);
    asm.label("weave_keepday");
    asm.invoke(
        "static",
        method("Ljava/lang/Integer;", "valueOf", "Ljava/lang/Integer;", &["I"]),
        &[10],
    );
    asm.move_result(14, true);
    asm.aput_object(14, 4, 9);
    asm.array_length(10, 3);
    asm.if_cmp("ge", 9, 10, "weave_noend");
    asm.aget_object(10, 3, 9);
    asm.invoke(
        "static",
        method(rt, "dateKey", "I", &["Ljava/lang/String;"]),
        &[10],
    );
    asm.move_result(10, false);
    asm.goto("weave_boxed");
    asm.label("weave_noend");
    int_zero(asm, 10);
    asm.label("weave_boxed");
    asm.invoke(
        "static",
        method("Ljava/lang/Integer;", "valueOf", "Ljava/lang/Integer;", &["I"]),
        &[10],
    );
    asm.move_result(14, true);
    asm.aput_object(14, 5, 9);
    asm.const_(10, 0);
    asm.invoke(
        "static",
        method("Ljava/lang/Integer;", "valueOf", "Ljava/lang/Integer;", &["I"]),
        &[10],
    );
    asm.move_result(14, true);
    asm.aput_object(14, 6, 9);
    asm.const_(10, 1);
    asm.binop(0x90, 9, 9, 10, "add-int");
    asm.goto("weave_fill");
    asm.label("weave_filled");
    asm.new_instance(7, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        method("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[7],
    );
    int_zero(asm, 12);
    int_zero(asm, 9);
    int_zero(asm, 14);
    int_zero(asm, 15);
    asm.label("weave_walk");
    asm.if_cmp("ge", 9, 8, "weave_last");
    asm.aget_object(14, 4, 9);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Integer;", "intValue", "I", &[]),
        &[14],
    );
    asm.move_result(14, false);
    int_zero(asm, 10);
    int_zero(asm, 15);
    asm.if_cmp("eq", 14, 12, "weave_note");
    asm.if_cmp("eq", 12, 10, "weave_gap");
    emit_weave_flush(asm, "eq", "leave");
    asm.aget_object(14, 4, 9);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Integer;", "intValue", "I", &[]),
        &[14],
    );
    asm.move_result(14, false);
    asm.label("weave_gap");
    asm.move_(13, 14, false);
    emit_weave_flush(asm, "between", "gap");
    asm.aget_object(14, 4, 9);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Integer;", "intValue", "I", &[]),
        &[14],
    );
    asm.move_result(14, false);
    asm.label("weave_note");
    emit_weave_field(asm, false);
    asm.aget_object(14, 4, 9);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Integer;", "intValue", "I", &[]),
        &[14],
    );
    asm.move_result(14, false);
    asm.move_(12, 14, false);
    asm.const_(10, 1);
    asm.binop(0x90, 9, 9, 10, "add-int");
    int_zero(asm, 15);
    asm.goto("weave_walk");
    asm.label("weave_last");
    int_zero(asm, 10);
    asm.if_cmp("eq", 12, 10, "weave_out");
    asm.const_(13, 12);
    emit_weave_flush(asm, "eq", "last");
    asm.const_(13, 2_147_483_647);
    emit_weave_flush(asm, "between", "tail");
    asm.label("weave_out");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[7],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

/// Emit end rows whose end key is `v12` (`eq`) or strictly between `v12` and `v13`.
fn emit_weave_flush(asm: &mut Asm, kind: &str, site: &str) {
    let head = format!("fl_{kind}_{site}");
    let skip = format!("fs_{kind}_{site}");
    let done = format!("fd_{kind}_{site}");
    int_zero(asm, 10);
    int_zero(asm, 14);
    int_zero(asm, 15);
    asm.label(&head);
    asm.if_cmp("ge", 10, 8, &done);
    asm.aget_object(15, 6, 10);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Integer;", "intValue", "I", &[]),
        &[15],
    );
    asm.move_result(15, false);
    int_zero(asm, 14);
    asm.if_cmp("ne", 15, 14, &skip);
    asm.aget_object(15, 5, 10);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Integer;", "intValue", "I", &[]),
        &[15],
    );
    asm.move_result(15, false);
    int_zero(asm, 14);
    asm.if_cmp("eq", 15, 14, &skip);
    if kind == "eq" {
        asm.if_cmp("ne", 15, 12, &skip);
    } else {
        asm.if_cmp("le", 15, 12, &skip);
        asm.if_cmp("ge", 15, 13, &skip);
    }
    asm.const_(14, 1);
    asm.invoke(
        "static",
        method("Ljava/lang/Integer;", "valueOf", "Ljava/lang/Integer;", &["I"]),
        &[14],
    );
    asm.move_result(15, true);
    asm.aput_object(15, 6, 10);
    int_zero(asm, 14);
    asm.if_cmp("ne", 11, 14, &format!("f1_{kind}_{site}"));
    asm.aget_object(15, 0, 10);
    asm.goto(&format!("fadd_{kind}_{site}"));
    asm.label(&format!("f1_{kind}_{site}"));
    asm.const_(14, 1);
    asm.if_cmp("ne", 11, 14, &format!("f2_{kind}_{site}"));
    emit_at(asm, 3, 10, 15, &format!("end_{kind}_{site}"));
    asm.goto(&format!("fadd_{kind}_{site}"));
    asm.label(&format!("f2_{kind}_{site}"));
    asm.const_string(15, "PeriodEnd");
    asm.label(&format!("fadd_{kind}_{site}"));
    emit_weave_append(asm, 15, &format!("{kind}_{site}"));
    int_zero(asm, 15);
    int_zero(asm, 14);
    asm.label(&skip);
    asm.const_(14, 1);
    asm.binop(0x90, 10, 10, 14, "add-int");
    asm.goto(&head);
    asm.label(&done);
}

fn emit_at(asm: &mut Asm, array: u16, index: u16, dest: u16, tag: &str) {
    let miss = format!("at_m_{tag}");
    let hit = format!("at_h_{tag}");
    asm.array_length(dest, array);
    asm.if_cmp("ge", index, dest, &miss);
    asm.aget_object(dest, array, index);
    asm.goto(&hit);
    asm.label(&miss);
    asm.const_string(dest, "");
    asm.label(&hit);
}

fn emit_weave_field(asm: &mut Asm, _end: bool) {
    int_zero(asm, 14);
    asm.if_cmp("ne", 11, 14, "field_day");
    emit_at(asm, 0, 9, 15, "title");
    asm.goto("field_add");
    asm.label("field_day");
    asm.const_(14, 1);
    asm.if_cmp("ne", 11, 14, "field_mark");
    emit_at(asm, 1, 9, 15, "day");
    asm.goto("field_add");
    asm.label("field_mark");
    emit_at(asm, 2, 9, 15, "mark");
    asm.label("field_add");
    emit_weave_append(asm, 15, "note");
}

fn emit_weave_append(asm: &mut Asm, text: u16, tag: &str) {
    let bare = format!("apb_{tag}");
    let ready = format!("apr_{tag}");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[7],
    );
    asm.move_result(14, false);
    asm.if_zero("eqz", 14, &bare);
    asm.const_string(14, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[7, 14],
    );
    asm.move_result(7, true);
    int_zero(asm, 14);
    asm.goto(&ready);
    asm.label(&bare);
    int_zero(asm, 14);
    asm.label(&ready);
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[7, text],
    );
    asm.move_result(7, true);
}

fn emit_journal_items(asm: &mut Asm, rt: &str) {
    emit_journal(asm, false, rt);
}

fn emit_journal_details(asm: &mut Asm, rt: &str) {
    emit_journal(asm, true, rt);
}

/// A day stored as `2026 November 11` becomes a year line, a month line, and
/// `Wednesday, November 11`. The same day is not repeated. One undated string stays one line.
///
/// Items use registers 18, so the text is `v16` and the days are `v17`.
/// Details use registers 20: text `v17`, days `v18`, marks `v19`, and the
/// mark array is `v16`. Heading scratch is `v0`–`v15` in both.
fn emit_journal(asm: &mut Asm, details: bool, rt: &str) {
    // 16 registers. Items: text v14, days v15. Details: text v13, days v14, marks v15.
    // After the splits, v13 is the mark array in the details method and v14/v15 are free.
    let titles: u16 = if details { 13 } else { 14 };
    let days: u16 = if details { 14 } else { 15 };
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[titles],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "empty");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[titles, 0],
    );
    asm.move_result(2, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[days],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "nodays");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[days, 0],
    );
    asm.move_result(1, true);
    asm.const_(0, 0);
    asm.goto("havedays");
    asm.label("nodays");
    asm.const_(0, 0);
    asm.new_array(1, 0, "[Ljava/lang/String;");
    asm.label("havedays");
    if details {
        asm.invoke(
            "virtual",
            method("Ljava/lang/String;", "length", "I", &[]),
            &[15],
        );
        asm.move_result(0, false);
        asm.if_zero("eqz", 0, "nomarks");
        asm.const_string(0, "\n");
        asm.invoke(
            "virtual",
            method(
                "Ljava/lang/String;",
                "split",
                "[Ljava/lang/String;",
                &["Ljava/lang/String;"],
            ),
            &[15, 0],
        );
        asm.move_result(13, true);
        asm.const_(0, 0);
        asm.goto("havemarks");
        asm.label("nomarks");
        asm.const_(0, 0);
        asm.new_array(13, 0, "[Ljava/lang/String;");
        asm.label("havemarks");
    }
    asm.array_length(3, 2);
    asm.array_length(7, 1);
    asm.new_instance(4, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        method("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[4],
    );
    asm.const_string(6, "");
    asm.const_string(9, "");
    asm.const_string(10, "");
    asm.const_string(11, "");
    asm.const_string(12, "");
    asm.const_string(14, "");
    asm.const_string(15, "");
    asm.const_string(0, "");
    asm.const_(5, 0);
    asm.const_(8, 0);
    asm.label("loop");
    asm.if_cmp("ge", 5, 3, "done");
    asm.if_cmp("ge", 5, 7, "row");
    asm.aget_object(0, 1, 5);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[0],
    );
    asm.move_result(8, false);
    asm.if_zero("eqz", 8, "row");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "equals", "Z", &["Ljava/lang/Object;"]),
        &[0, 6],
    );
    asm.move_result(8, false);
    asm.if_zero("nez", 8, "row");
    asm.move_(6, 0, true);
    asm.const_(8, 32);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "indexOf", "I", &["I"]),
        &[6, 8],
    );
    asm.move_result(8, false);
    asm.const_(3, 0);
    asm.if_cmp("lt", 8, 3, "whole");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I", "I"]),
        &[6, 3, 8],
    );
    asm.move_result(11, true);
    asm.const_(3, 1);
    asm.binop(0x90, 8, 8, 3, "add-int");
    asm.array_length(3, 2);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I"]),
        &[6, 8],
    );
    asm.move_result(12, true);
    asm.const_(8, 32);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "indexOf", "I", &["I"]),
        &[12, 8],
    );
    asm.move_result(8, false);
    asm.const_(3, 0);
    asm.if_cmp("lt", 8, 3, "ym");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I", "I"]),
        &[12, 3, 8],
    );
    asm.move_result(15, true);
    asm.const_(3, 1);
    asm.binop(0x90, 8, 8, 3, "add-int");
    asm.array_length(3, 2);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I"]),
        &[12, 8],
    );
    asm.move_result(14, true);
    asm.move_(12, 15, true);
    emit_changed(asm, 11, 9, "Year", details, "y");
    emit_changed(asm, 12, 10, "Month", details, "m");
    if details {
        asm.const_string(15, "Day");
    } else {
        asm.invoke(
            "static",
            method(
                rt,
                "dayHeading",
                "Ljava/lang/String;",
                &["Ljava/lang/String;", "Ljava/lang/String;", "Ljava/lang/String;"],
            ),
            &[11, 12, 14],
        );
        asm.move_result(15, true);
    }
    append_line(asm, 15, "d");
    asm.goto("row");
    asm.label("ym");
    asm.array_length(3, 2);
    emit_changed(asm, 11, 9, "Year", details, "u");
    emit_changed(asm, 12, 10, "Month", details, "n");
    asm.goto("row");
    asm.label("whole");
    asm.array_length(3, 2);
    if details {
        asm.const_string(11, "Day");
        append_line(asm, 11, "w");
    } else {
        append_line(asm, 6, "w");
    }
    asm.const_string(9, "");
    asm.const_string(10, "");
    asm.label("row");
    asm.const_(8, 0);
    asm.const_string(0, "");
    if details {
        asm.array_length(8, 13);
        asm.if_cmp("ge", 5, 8, "blank");
        asm.aget_object(11, 13, 5);
        asm.goto("write");
        asm.label("blank");
        asm.const_string(11, "");
        asm.label("write");
    } else {
        asm.aget_object(11, 2, 5);
    }
    append_line(asm, 11, "r");
    asm.const_(8, 1);
    asm.binop(0x90, 5, 5, 8, "add-int");
    asm.const_(8, 0);
    asm.const_string(0, "");
    asm.goto("loop");
    asm.label("done");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[4],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("empty");
    asm.const_string(0, "");
    asm.ret(0, "obj");
}
fn emit_line_index(asm: &mut Asm) {
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[6],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "empty");
    asm.const_string(1, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[6, 1],
    );
    asm.move_result(2, true);
    asm.array_length(3, 2);
    asm.const_(4, 0);
    asm.const_(0, 1);
    asm.label("loop");
    asm.if_cmp("ge", 4, 3, "none");
    asm.aget_object(5, 2, 4);
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "equals",
            "Z",
            &["Ljava/lang/Object;"],
        ),
        &[5, 7],
    );
    asm.move_result(5, false);
    asm.if_zero("nez", 5, "hit");
    asm.binop(0x90, 4, 4, 0, "add-int");
    asm.goto("loop");
    asm.label("hit");
    asm.ret(4, "int");
    asm.label("none");
    asm.const_(0, -1);
    asm.ret(0, "int");
    asm.label("empty");
    asm.const_(0, -1);
    asm.ret(0, "int");
}

/// `v6` is the text and `v7` the index. A bad index returns an empty string.
fn emit_line_at(asm: &mut Asm) {
    asm.const_(0, 0);
    asm.if_cmp("lt", 7, 0, "bad");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[6],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "bad");
    asm.const_string(1, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[6, 1],
    );
    asm.move_result(2, true);
    asm.array_length(3, 2);
    asm.if_cmp("ge", 7, 3, "missing");
    asm.aget_object(0, 2, 7);
    asm.ret(0, "obj");
    asm.label("missing");
    asm.const_string(0, "");
    asm.ret(0, "obj");
    asm.label("bad");
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

/// `v6` is the text and `v7` the index. A bad index returns the text unchanged.
/// `v7` text, `v8` index, `v9` the replacement line. A bad index returns the text.
fn emit_replace_at(asm: &mut Asm) {
    asm.const_(0, 0);
    asm.if_cmp("lt", 8, 0, "low");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[7],
    );
    asm.move_result(1, false);
    asm.if_zero("eqz", 1, "blank");
    asm.const_string(1, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[7, 1],
    );
    asm.move_result(1, true);
    asm.array_length(2, 1);
    asm.if_cmp("ge", 8, 2, "high");
    asm.new_instance(3, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        method("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[3],
    );
    asm.const_(0, 0);
    asm.const_(4, 0);
    asm.const_(5, 0);
    asm.label("loop");
    asm.if_cmp("ge", 4, 2, "done");
    asm.if_cmp("eq", 4, 8, "put");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[3],
    );
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "copy");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[3, 0],
    );
    asm.move_result(3, true);
    asm.const_(0, 0);
    asm.label("copy");
    asm.aget_object(0, 1, 4);
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[3, 0],
    );
    asm.move_result(3, true);
    asm.const_(0, 0);
    asm.goto("next");
    asm.label("put");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[3],
    );
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "putline");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[3, 0],
    );
    asm.move_result(3, true);
    asm.const_(0, 0);
    asm.label("putline");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[3, 9],
    );
    asm.move_result(3, true);
    asm.const_(0, 0);
    asm.label("next");
    asm.const_(5, 1);
    asm.binop(0x90, 4, 4, 5, "add-int");
    asm.goto("loop");
    asm.label("done");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[3],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("high");
    asm.ret(7, "obj");
    asm.label("blank");
    asm.ret(7, "obj");
    asm.label("low");
    asm.ret(7, "obj");
}

fn emit_drop_at(asm: &mut Asm) {
    asm.const_(0, 0);
    asm.if_cmp("lt", 7, 0, "low");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[6],
    );
    asm.move_result(1, false);
    asm.if_zero("eqz", 1, "blank");
    asm.const_string(1, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[6, 1],
    );
    asm.move_result(1, true);
    asm.array_length(2, 1);
    asm.if_cmp("ge", 7, 2, "high");
    asm.new_instance(3, "Ljava/lang/StringBuilder;");
    asm.invoke(
        "direct",
        method("Ljava/lang/StringBuilder;", "<init>", "V", &[]),
        &[3],
    );
    asm.const_(0, 0);
    asm.const_(4, 0);
    asm.const_(5, 0);
    asm.label("loop");
    asm.if_cmp("ge", 4, 2, "done");
    asm.if_cmp("eq", 4, 7, "next");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "length", "I", &[]),
        &[3],
    );
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "addpart");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[3, 0],
    );
    asm.move_result(3, true);
    asm.const_(0, 0);
    asm.label("addpart");
    asm.aget_object(0, 1, 4);
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/StringBuilder;",
            "append",
            "Ljava/lang/StringBuilder;",
            &["Ljava/lang/String;"],
        ),
        &[3, 0],
    );
    asm.move_result(3, true);
    asm.const_(0, 0);
    asm.label("next");
    asm.const_(5, 1);
    asm.binop(0x90, 4, 4, 5, "add-int");
    asm.goto("loop");
    asm.label("done");
    asm.invoke(
        "virtual",
        method("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", &[]),
        &[3],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
    asm.label("high");
    asm.ret(6, "obj");
    asm.label("blank");
    asm.ret(6, "obj");
    asm.label("low");
    asm.ret(6, "obj");
}

/// `v7` is the unused context. `yyyy MMMM d` is `2026 November 10`. `HH:mm` is the clock.
fn emit_stamp(asm: &mut Asm, pattern: &str) {
    asm.new_instance(0, "Ljava/text/SimpleDateFormat;");
    asm.const_string(1, pattern);
    asm.sget_object(
        2,
        field("Ljava/util/Locale;", "US", "Ljava/util/Locale;"),
    );
    asm.invoke(
        "direct",
        method(
            "Ljava/text/SimpleDateFormat;",
            "<init>",
            "V",
            &["Ljava/lang/String;", "Ljava/util/Locale;"],
        ),
        &[0, 1, 2],
    );
    asm.new_instance(1, "Ljava/util/Date;");
    asm.invoke(
        "direct",
        method("Ljava/util/Date;", "<init>", "V", &[]),
        &[1],
    );
    asm.invoke(
        "virtual",
        method(
            "Ljava/text/DateFormat;",
            "format",
            "Ljava/lang/String;",
            &["Ljava/util/Date;"],
        ),
        &[0, 1],
    );
    asm.move_result(0, true);
    asm.ret(0, "obj");
}

fn emit_paste_text(asm: &mut Asm) {
    asm.const_string(0, "clipboard");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "getSystemService",
            "Ljava/lang/Object;",
            &["Ljava/lang/String;"],
        ),
        &[7, 0],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, "none");
    asm.check_cast(0, "Landroid/content/ClipboardManager;");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/ClipboardManager;",
            "hasPrimaryClip",
            "Z",
            &[],
        ),
        &[0],
    );
    asm.move_result(1, false);
    asm.if_zero("eqz", 1, "none");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/ClipboardManager;",
            "getPrimaryClip",
            "Landroid/content/ClipData;",
            &[],
        ),
        &[0],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, "none");
    asm.invoke(
        "virtual",
        method("Landroid/content/ClipData;", "getItemCount", "I", &[]),
        &[2],
    );
    asm.move_result(1, false);
    asm.if_zero("eqz", 1, "none");
    asm.const_(1, 0);
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/ClipData;",
            "getItemAt",
            "Landroid/content/ClipData$Item;",
            &["I"],
        ),
        &[2, 1],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, "none");
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/ClipData$Item;",
            "getText",
            "Ljava/lang/CharSequence;",
            &[],
        ),
        &[2],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, "none");
    asm.invoke(
        "virtual",
        method("Ljava/lang/Object;", "toString", "Ljava/lang/String;", &[]),
        &[2],
    );
    asm.move_result(2, true);
    asm.ret(2, "obj");
    asm.label("none");
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

/// `View.setTag(int, Object)` rejects a key whose top byte is below 2.
/// `0x7f` is the application package id, so these pass on API 24 and on current devices.
const TAG_ITEMS: i64 = 0x7f01_0001;
const TAG_DETAILS: i64 = 0x7f01_0002;

/// Items live under `TAG_ITEMS` and details under `TAG_DETAILS`. A missing tag reads as "".
fn emit_load_tag(asm: &mut Asm, key: i64, miss: &str, got: &str) {
    asm.const_(1, key);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "getTag", "Ljava/lang/Object;", &["I"]),
        &[5, 1],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, miss);
    asm.check_cast(0, "Ljava/lang/String;");
    asm.goto(got);
    asm.label(miss);
    asm.const_string(0, "");
    asm.label(got);
}

fn emit_store_tags(asm: &mut Asm, items: u16, details: u16) {
    asm.const_(1, TAG_ITEMS);
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setTag",
            "V",
            &["I", "Ljava/lang/Object;"],
        ),
        &[5, 1, items],
    );
    asm.const_(1, TAG_DETAILS);
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setTag",
            "V",
            &["I", "Ljava/lang/Object;"],
        ),
        &[5, 1, details],
    );
}

/// `String.split` returns `String[]`. An empty string becomes a zero-length array,
/// not one blank row. `len` stays an int on both sides of the join; `delim` does not.
fn emit_split(asm: &mut Asm, text: u16, dest: u16, len: u16, delim: u16, prefix: &str) {
    let have = format!("{prefix}_have");
    let ready = format!("{prefix}_ready");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[text],
    );
    asm.move_result(len, false);
    asm.if_zero("nez", len, &have);
    asm.const_(len, 0);
    asm.new_array(dest, len, "[Ljava/lang/String;");
    asm.goto(&ready);
    asm.label(&have);
    asm.const_string(delim, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[text, delim],
    );
    asm.move_result(dest, true);
    asm.label(&ready);
}

/// `android.R.layout.simple_list_item_1` is 0x01090003. The root view is a TextView,
/// so the three-argument ArrayAdapter constructor can fill it.
fn emit_one_line(asm: &mut Asm) {
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[7],
    );
    asm.move_result(0, false);
    asm.if_zero("nez", 0, "lines_have");
    asm.const_(3, 0);
    asm.new_array(0, 3, "[Ljava/lang/String;");
    asm.goto("lines_ready");
    asm.label("lines_have");
    asm.const_string(1, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[7, 1],
    );
    asm.move_result(0, true);
    asm.label("lines_ready");
    asm.check_cast(0, "[Ljava/lang/Object;");
    asm.const_(2, 0x01090003);
    asm.new_instance(1, "Landroid/widget/ArrayAdapter;");
    asm.invoke(
        "direct",
        method(
            "Landroid/widget/ArrayAdapter;",
            "<init>",
            "V",
            &[
                "Landroid/content/Context;",
                "I",
                "[Ljava/lang/Object;",
            ],
        ),
        &[1, 6, 2, 0],
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/AbsListView;",
            "setAdapter",
            "V",
            &["Landroid/widget/ListAdapter;"],
        ),
        &[5, 1],
    );
    asm.ret(0, "void");
}

/// `simple_list_item_2` is 0x01090004. Its root is not a TextView, so DroideRows
/// passes `android.R.id.text1` to the superclass and fills `text2` itself.
fn emit_two_line(asm: &mut Asm, rows_desc: &str, items: u16, details: u16) {
    emit_split(asm, items, 2, 1, 4, "ia");
    emit_split(asm, details, 3, 1, 4, "da");
    asm.check_cast(2, "[Ljava/lang/Object;");
    asm.const_(1, 0x01090004);
    asm.new_instance(4, rows_desc);
    asm.invoke(
        "direct",
        method(
            rows_desc,
            "<init>",
            "V",
            &[
                "Landroid/content/Context;",
                "I",
                "[Ljava/lang/Object;",
                "[Ljava/lang/String;",
            ],
        ),
        &[4, 6, 1, 2, 3],
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/AbsListView;",
            "setAdapter",
            "V",
            &["Landroid/widget/ListAdapter;"],
        ),
        &[5, 4],
    );
    asm.new_instance(0, "Landroid/graphics/drawable/ColorDrawable;");
    asm.const_(1, argb(0x00F3_EFE6));
    asm.invoke(
        "direct",
        method(
            "Landroid/graphics/drawable/ColorDrawable;",
            "<init>",
            "V",
            &["I"],
        ),
        &[0, 1],
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/ListView;",
            "setDivider",
            "V",
            &["Landroid/graphics/drawable/Drawable;"],
        ),
        &[5, 0],
    );
    asm.const_(1, 18);
    asm.invoke(
        "virtual",
        method("Landroid/widget/ListView;", "setDividerHeight", "V", &["I"]),
        &[5, 1],
    );
    asm.new_instance(0, "Landroid/graphics/drawable/ColorDrawable;");
    asm.const_(1, 0);
    asm.invoke(
        "direct",
        method(
            "Landroid/graphics/drawable/ColorDrawable;",
            "<init>",
            "V",
            &["I"],
        ),
        &[0, 1],
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/AbsListView;",
            "setSelector",
            "V",
            &["Landroid/graphics/drawable/Drawable;"],
        ),
        &[5, 0],
    );
    asm.ret(0, "void");
}

/// `v5` is the list, `v6` the context, and `v7` the new items. Empty details keep
/// the one-line row. Items define how many rows exist.
fn emit_set_lines(asm: &mut Asm, rows_desc: &str) {
    emit_load_tag(asm, TAG_DETAILS, "det_miss", "det_got");
    emit_store_tags(asm, 7, 0);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[0],
    );
    asm.move_result(1, false);
    asm.if_zero("nez", 1, "two_lines");
    emit_one_line(asm);
    asm.label("two_lines");
    emit_two_line(asm, rows_desc, 7, 0);
}

/// `v7` is the new details. The stored items stay the row titles.
fn emit_set_details(asm: &mut Asm, rows_desc: &str) {
    emit_load_tag(asm, TAG_ITEMS, "item_miss", "item_got");
    emit_store_tags(asm, 0, 7);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[7],
    );
    asm.move_result(1, false);
    asm.if_zero("nez", 1, "two_lines");
    asm.move_(7, 0, true);
    emit_one_line(asm);
    asm.label("two_lines");
    emit_two_line(asm, rows_desc, 0, 7);
}

/// `v3` is the view. `TAG_ITEMS` is the row text and `TAG_DETAILS` is the second line.
fn emit_tagged_string(asm: &mut Asm, key: i64) {
    asm.const_(0, key);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "getTag", "Ljava/lang/Object;", &["I"]),
        &[3, 0],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, "none");
    asm.check_cast(0, "Ljava/lang/String;");
    asm.ret(0, "obj");
    asm.label("none");
    asm.const_string(0, "");
    asm.ret(0, "obj");
}

/// `v1` is this, `v2` the context, `v3` the layout, `v4` the titles, `v5` the
/// details. `v0` holds `android.R.id.text1` (0x01020014).
fn emit_rows_init(asm: &mut Asm, rows_desc: &str) {
    asm.const_(0, 0x01020014);
    asm.invoke(
        "direct",
        method(
            "Landroid/widget/ArrayAdapter;",
            "<init>",
            "V",
            &[
                "Landroid/content/Context;",
                "I",
                "I",
                "[Ljava/lang/Object;",
            ],
        ),
        &[1, 2, 3, 0, 4],
    );
    asm.iput_obj(5, 1, field(rows_desc, "details", "[Ljava/lang/String;"));
    asm.const_(0, argb(0x00F3_EFE6));
    asm.iput(0, 1, field(rows_desc, "paper", "I"));
    asm.const_(0, argb(0x00FB_F8F2));
    asm.iput(0, 1, field(rows_desc, "card", "I"));
    asm.const_(0, argb(0x001C_1915));
    asm.iput(0, 1, field(rows_desc, "ink", "I"));
    asm.const_(0, argb(0x006E_665C));
    asm.iput(0, 1, field(rows_desc, "muted", "I"));
    asm.const_(0, 0);
    asm.iput(0, 1, field(rows_desc, "rowSize", "I"));
    asm.const_string(0, "");
    asm.iput_obj(0, 1, field(rows_desc, "picked", "Ljava/lang/String;"));
    asm.ret(0, "void");
}

fn argb(rgb: u32) -> i64 {
    (0xFF00_0000 | rgb) as i32 as i64
}

/// A square button is a round plaque. `v2` is the button. Padding is cleared so the word fits.
fn emit_round_button(
    asm: &mut Asm,
    props: Option<&Map<String, Value>>,
) -> Result<(), CompileError> {
    let raw = props
        .and_then(|map| map.get("Color"))
        .map(py_str)
        .unwrap_or_else(|| "#FBF8F2".to_string());
    let color = parse_color(&Value::String(raw)).map_err(CompileError::bare)? as i64;
    asm.new_instance(3, "Landroid/graphics/drawable/GradientDrawable;");
    asm.invoke(
        "direct",
        method(
            "Landroid/graphics/drawable/GradientDrawable;",
            "<init>",
            "V",
            &[],
        ),
        &[3],
    );
    asm.const_(4, 1);
    asm.invoke(
        "virtual",
        method(
            "Landroid/graphics/drawable/GradientDrawable;",
            "setShape",
            "V",
            &["I"],
        ),
        &[3, 4],
    );
    asm.const_(4, color);
    asm.invoke(
        "virtual",
        method(
            "Landroid/graphics/drawable/GradientDrawable;",
            "setColor",
            "V",
            &["I"],
        ),
        &[3, 4],
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setBackground",
            "V",
            &["Landroid/graphics/drawable/Drawable;"],
        ),
        &[2, 3],
    );
    asm.const_(4, 0);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setPadding", "V", &["I", "I", "I", "I"]),
        &[2, 4, 4, 4, 4],
    );
    Ok(())
}

/// `v1` is the detail. A matching word draws a plain heading and jumps to `done`.
fn emit_heading_gate(
    asm: &mut Asm,
    rows: &str,
    word: &str,
    next: &str,
    size: i64,
    tone: &str,
    min_h: i64,
) {
    asm.const_string(6, word);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "equals", "Z", &["Ljava/lang/Object;"]),
        &[1, 6],
    );
    asm.move_result(3, false);
    asm.if_zero("eqz", 3, next);
    emit_heading_row(asm, rows, word, size, tone, min_h);
    asm.const_string(1, "");
    asm.goto("done");
    asm.label(next);
}

/// A year, month, or day line. Not a card. `v5` is the row index and is restored.
fn emit_heading_row(asm: &mut Asm, rows: &str, tag: &str, size: i64, tone: &str, min_h: i64) {
    let typed = format!("hty{tag}");
    let done = format!("hdn{tag}");
    asm.const_(1, 8);
    asm.const_(2, 8);
    asm.const_(3, 4);
    asm.const_(6, 2);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setPadding", "V", &["I", "I", "I", "I"]),
        &[0, 1, 3, 2, 6],
    );
    asm.iget(3, 4, field(rows, "paper", "I"));
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setBackgroundColor", "V", &["I"]),
        &[0, 3],
    );
    asm.const_(3, min_h);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setMinimumHeight", "V", &["I"]),
        &[0, 3],
    );
    asm.const_(3, 0x01020014);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "findViewById", "Landroid/view/View;", &["I"]),
        &[0, 3],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, &typed);
    asm.check_cast(2, "Landroid/widget/TextView;");
    asm.iget(3, 4, field(rows, tone, "I"));
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setTextColor", "V", &["I"]),
        &[2, 3],
    );
    asm.const_(3, size);
    asm.unop(0x82, 3, 3, "int-to-float");
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setTextSize", "V", &["F"]),
        &[2, 3],
    );
    asm.const_(3, 3);
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setGravity", "V", &["I"]),
        &[2, 3],
    );
    asm.label(&typed);
    asm.const_(3, 0x01020015);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "findViewById", "Landroid/view/View;", &["I"]),
        &[0, 3],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, &done);
    asm.const_(3, 8);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setVisibility", "V", &["I"]),
        &[2, 3],
    );
    asm.label(&done);
}

/// The day a period stops. A thin rule, with no words.
fn emit_end_row(asm: &mut Asm, rows: &str) {
    asm.const_(6, 0);
    asm.iget(3, 4, field(rows, "ink", "I"));
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setBackgroundColor", "V", &["I"]),
        &[0, 3],
    );
    asm.const_(3, 8);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setMinimumHeight", "V", &["I"]),
        &[0, 3],
    );
    for (id, tag) in [(0x01020014i64, "a"), (0x01020015, "b")] {
        asm.const_(3, id);
        asm.invoke(
            "virtual",
            method("Landroid/view/View;", "findViewById", "Landroid/view/View;", &["I"]),
            &[0, 3],
        );
        asm.move_result(2, true);
        asm.if_zero("eqz", 2, &format!("end_hide_{tag}"));
        asm.const_(3, 8);
        asm.invoke(
            "virtual",
            method("Landroid/view/View;", "setVisibility", "V", &["I"]),
            &[2, 3],
        );
        asm.label(&format!("end_hide_{tag}"));
    }
}

/// A Period is a full-width rule with its words centered, not a card.
fn emit_period_row(asm: &mut Asm, rows: &str) {
    asm.const_(1, 12);
    asm.const_(2, 8);
    asm.const_(3, 12);
    asm.const_(6, 8);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setPadding", "V", &["I", "I", "I", "I"]),
        &[0, 1, 2, 3, 6],
    );
    asm.iget(3, 4, field(rows, "paper", "I"));
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setBackgroundColor", "V", &["I"]),
        &[0, 3],
    );
    asm.const_(3, 0x01020014);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "findViewById", "Landroid/view/View;", &["I"]),
        &[0, 3],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, "period_type");
    asm.check_cast(2, "Landroid/widget/TextView;");
    asm.iget(3, 4, field(rows, "ink", "I"));
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setTextColor", "V", &["I"]),
        &[2, 3],
    );
    asm.const_(3, 17);
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setGravity", "V", &["I"]),
        &[2, 3],
    );
    asm.label("period_type");
    asm.const_(3, 0x01020015);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "findViewById", "Landroid/view/View;", &["I"]),
        &[0, 3],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, "period_done");
    asm.const_(3, 8);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setVisibility", "V", &["I"]),
        &[2, 3],
    );
    asm.label("period_done");
}

fn emit_row_card(asm: &mut Asm, rows: &str) {
    asm.const_(1, 28);
    asm.const_(2, 20);
    asm.const_(3, 28);
    asm.const_(6, 16);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setPadding", "V", &["I", "I", "I", "I"]),
        &[0, 1, 2, 3, 6],
    );
    asm.new_instance(1, "Landroid/graphics/drawable/GradientDrawable;");
    asm.invoke(
        "direct",
        method(
            "Landroid/graphics/drawable/GradientDrawable;",
            "<init>",
            "V",
            &[],
        ),
        &[1],
    );
    asm.iget(3, 4, field(rows, "card", "I"));
    asm.invoke(
        "virtual",
        method(
            "Landroid/graphics/drawable/GradientDrawable;",
            "setColor",
            "V",
            &["I"],
        ),
        &[1, 3],
    );
    asm.const_(3, 24);
    asm.unop(0x82, 3, 3, "int-to-float");
    asm.invoke(
        "virtual",
        method(
            "Landroid/graphics/drawable/GradientDrawable;",
            "setCornerRadius",
            "V",
            &["F"],
        ),
        &[1, 3],
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setBackground",
            "V",
            &["Landroid/graphics/drawable/Drawable;"],
        ),
        &[0, 1],
    );
}

/// `v4` is this, `v5` the position, `v6` the convert view, and `v7` the parent.
/// `v3` stays an int (the length, then an id) so it can join at `done`.
/// `v2` is a text view. A missing line leaves the type line untouched.
fn emit_rows_view(asm: &mut Asm, rows_desc: &str) {
    asm.invoke(
        "super",
        method(
            "Landroid/widget/ArrayAdapter;",
            "getView",
            "Landroid/view/View;",
            &[
                "I",
                "Landroid/view/View;",
                "Landroid/view/ViewGroup;",
            ],
        ),
        &[4, 5, 6, 7],
    );
    asm.move_result(0, true);
    asm.iget_obj(1, 4, field(rows_desc, "details", "[Ljava/lang/String;"));
    asm.if_zero("eqz", 1, "plain");
    asm.array_length(3, 1);
    asm.if_cmp("ge", 5, 3, "plain");
    asm.aget_object(1, 1, 5);
    asm.if_zero("eqz", 1, "plain");
    emit_heading_gate(asm, rows_desc, "Year", "notyear", 28, "ink", 44);
    emit_heading_gate(asm, rows_desc, "Month", "notmonth", 20, "ink", 32);
    emit_heading_gate(asm, rows_desc, "Day", "notday", 14, "muted", 24);
    asm.const_string(6, "PeriodEnd");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[1, 6],
    );
    asm.move_result(3, false);
    asm.if_zero("eqz", 3, "notend");
    emit_end_row(asm, rows_desc);
    asm.const_string(1, "");
    asm.goto("done");
    asm.label("notend");
    asm.const_string(6, "Period");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[1, 6],
    );
    asm.move_result(3, false);
    asm.if_zero("eqz", 3, "plain");
    emit_period_row(asm, rows_desc);
    asm.const_string(1, "");
    asm.goto("done");
    asm.label("plain");
    emit_row_card(asm, rows_desc);
    asm.const_(3, 0x01020014);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "findViewById", "Landroid/view/View;", &["I"]),
        &[0, 3],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, "title_skip");
    asm.check_cast(2, "Landroid/widget/TextView;");
    asm.iget(3, 4, field(rows_desc, "ink", "I"));
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setTextColor", "V", &["I"]),
        &[2, 3],
    );
    asm.const_(6, 0);
    asm.iget(3, 4, field(rows_desc, "rowSize", "I"));
    asm.if_zero("eqz", 3, "title_sized");
    asm.unop(0x82, 6, 3, "int-to-float");
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setTextSize", "V", &["F"]),
        &[2, 6],
    );
    asm.const_(6, 0);
    asm.label("title_sized");
    // ANTI_ALIAS, and not strike-through. A recycled done-task must not stay crossed out.
    asm.const_(6, 1);
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setPaintFlags", "V", &["I"]),
        &[2, 6],
    );
    asm.label("title_skip");
    asm.iget_obj(1, 4, field(rows_desc, "details", "[Ljava/lang/String;"));
    asm.if_zero("eqz", 1, "done");
    asm.array_length(3, 1);
    asm.if_cmp("ge", 5, 3, "done");
    asm.aget_object(1, 1, 5);
    asm.if_zero("eqz", 1, "done");
    asm.const_(3, 0x01020015);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "findViewById", "Landroid/view/View;", &["I"]),
        &[0, 3],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, "done");
    asm.check_cast(2, "Landroid/widget/TextView;");
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/TextView;",
            "setText",
            "V",
            &["Ljava/lang/CharSequence;"],
        ),
        &[2, 1],
    );
    asm.iget(3, 4, field(rows_desc, "muted", "I"));
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setTextColor", "V", &["I"]),
        &[2, 3],
    );
    // RIGHT | BOTTOM. The type sits on the lower right of the card.
    asm.const_(3, 85);
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setGravity", "V", &["I"]),
        &[2, 3],
    );
    asm.iget(6, 4, field(rows_desc, "muted", "I"));
    asm.invoke(
        "static",
        method(
            &rows_desc.replace("DroideRows", "DroideRt"),
            "paintTask",
            "V",
            &["Landroid/view/View;", "Ljava/lang/String;", "I"],
        ),
        &[0, 1, 6],
    );
    asm.label("done");
    asm.invoke(
        "static",
        method(
            &rows_desc.replace("DroideRows", "DroideRt"),
            "paintSpan",
            "V",
            &["Landroid/view/View;", rows_desc, "I"],
        ),
        &[0, 4, 5],
    );
    asm.ret(0, "obj");
}

fn tick_desc(package: &str) -> String {
    class_desc(package, "DroideTick")
}

fn timer_desc(package: &str, form: &str, name: &str) -> String {
    class_desc(package, &format!("{form}$Timer${name}"))
}

fn component_desc(package: &str, form: &str, kind: &str, name: &str, spec_desc: &str) -> String {
    if kind == "Timer" {
        timer_desc(package, form, name)
    } else {
        spec_desc.to_string()
    }
}

fn simple_class(desc: &str) -> &str {
    let body = desc.trim_start_matches('L').trim_end_matches(';');
    body.rsplit('/').next().unwrap_or(body)
}

/// `v5` is this. An interval below 1 waits 1 ms, so the loop cannot spin.
fn emit_tick_arm(asm: &mut Asm, tick: &str) {
    asm.iget(3, 5, field(tick, "interval", "I"));
    asm.const_(4, 1);
    asm.if_cmp("ge", 3, 4, "ok");
    asm.const_(3, 1);
    asm.label("ok");
    asm.unop(0x81, 0, 3, "int-to-long");
    asm.iget_obj(2, 5, field(tick, "handler", "Landroid/os/Handler;"));
    asm.invoke(
        "virtual",
        method(
            "Landroid/os/Handler;",
            "postDelayed",
            "Z",
            &["Ljava/lang/Runnable;", "J"],
        ),
        &[2, 5, 0, 1],
    );
    asm.move_result(3, false);
    asm.ret(0, "void");
}

fn emit_tick_disarm(asm: &mut Asm, tick: &str) {
    asm.iget_obj(0, 2, field(tick, "handler", "Landroid/os/Handler;"));
    asm.invoke(
        "virtual",
        method(
            "Landroid/os/Handler;",
            "removeCallbacks",
            "V",
            &["Ljava/lang/Runnable;"],
        ),
        &[0, 2],
    );
    asm.ret(0, "void");
}

/// `v4` is this and `v5` is the new interval. A running timer starts its wait over.
fn emit_tick_set_interval(asm: &mut Asm, tick: &str) {
    asm.const_(0, 1);
    asm.if_cmp("ge", 5, 0, "keep");
    asm.const_(5, 1);
    asm.label("keep");
    asm.iput(5, 4, field(tick, "interval", "I"));
    asm.iget(0, 4, field(tick, "enabled", "Z"));
    asm.if_zero("eqz", 0, "done");
    asm.invoke("virtual", method(tick, "disarm", "V", &[]), &[4]);
    asm.invoke("virtual", method(tick, "arm", "V", &[]), &[4]);
    asm.label("done");
    asm.ret(0, "void");
}

/// `v4` is this and `v5` is the new flag. Turning it on posts one wait. Turning
/// it off removes a pending tick. Setting it on again does not restart the wait.
fn emit_tick_set_enabled(asm: &mut Asm, tick: &str) {
    asm.iget(0, 4, field(tick, "enabled", "Z"));
    asm.iput(5, 4, field(tick, "enabled", "Z"));
    asm.if_zero("nez", 5, "turn_on");
    asm.invoke("virtual", method(tick, "disarm", "V", &[]), &[4]);
    asm.ret(0, "void");
    asm.label("turn_on");
    asm.if_zero("nez", 0, "stay");
    asm.invoke("virtual", method(tick, "arm", "V", &[]), &[4]);
    asm.label("stay");
    asm.ret(0, "void");
}

fn emit_tick_get(asm: &mut Asm, tick: &str, name: &str, ty: &str) {
    asm.iget(0, 1, field(tick, name, ty));
    asm.ret(0, "int");
}

fn emit_tick_init(asm: &mut Asm, tick: &str) {
    asm.invoke(
        "direct",
        method("Ljava/lang/Object;", "<init>", "V", &[]),
        &[4],
    );
    asm.invoke(
        "static",
        method("Landroid/os/Looper;", "getMainLooper", "Landroid/os/Looper;", &[]),
        &[],
    );
    asm.move_result(0, true);
    asm.new_instance(1, "Landroid/os/Handler;");
    asm.invoke(
        "direct",
        method(
            "Landroid/os/Handler;",
            "<init>",
            "V",
            &["Landroid/os/Looper;"],
        ),
        &[1, 0],
    );
    asm.iput_obj(1, 4, field(tick, "handler", "Landroid/os/Handler;"));
    asm.const_(0, 1000);
    asm.iput(0, 4, field(tick, "interval", "I"));
    asm.const_(0, 0);
    asm.iput(0, 4, field(tick, "enabled", "Z"));
    asm.ret(0, "void");
}

fn emit_tick_class(dex: &mut crate::dex::DexFile, package: &str) {
    let tick = tick_desc(package);
    let mut cls = ClassModel::new(&tick, "Ljava/lang/Object;", ACC_PUBLIC, "DroideTick");
    cls.interfaces.push("Ljava/lang/Runnable;".into());
    cls.field("handler", "Landroid/os/Handler;", ACC_PUBLIC);
    cls.field("interval", "I", ACC_PUBLIC);
    cls.field("enabled", "Z", ACC_PUBLIC);
    let mut init = Asm::new();
    emit_tick_init(&mut init, &tick);
    cls.method(MethodModel::new(
        "<init>",
        "V",
        Vec::new(),
        ACC_PUBLIC | ACC_CONSTRUCTOR,
        Some(init),
        5,
        1,
        2,
    ));
    let mut arm = Asm::new();
    emit_tick_arm(&mut arm, &tick);
    cls.method(MethodModel::new(
        "arm",
        "V",
        Vec::new(),
        ACC_PUBLIC,
        Some(arm),
        6,
        1,
        4,
    ));
    let mut disarm = Asm::new();
    emit_tick_disarm(&mut disarm, &tick);
    cls.method(MethodModel::new(
        "disarm",
        "V",
        Vec::new(),
        ACC_PUBLIC,
        Some(disarm),
        3,
        1,
        2,
    ));
    let mut set_interval = Asm::new();
    emit_tick_set_interval(&mut set_interval, &tick);
    cls.method(MethodModel::new(
        "setInterval",
        "V",
        vec!["I".into()],
        ACC_PUBLIC,
        Some(set_interval),
        6,
        2,
        1,
    ));
    let mut set_enabled = Asm::new();
    emit_tick_set_enabled(&mut set_enabled, &tick);
    cls.method(MethodModel::new(
        "setEnabled",
        "V",
        vec!["Z".into()],
        ACC_PUBLIC,
        Some(set_enabled),
        6,
        2,
        1,
    ));
    let mut interval = Asm::new();
    emit_tick_get(&mut interval, &tick, "interval", "I");
    cls.method(MethodModel::new(
        "interval",
        "I",
        Vec::new(),
        ACC_PUBLIC,
        Some(interval),
        2,
        1,
        1,
    ));
    let mut enabled = Asm::new();
    emit_tick_get(&mut enabled, &tick, "enabled", "Z");
    cls.method(MethodModel::new(
        "enabled",
        "Z",
        Vec::new(),
        ACC_PUBLIC,
        Some(enabled),
        2,
        1,
        1,
    ));
    // A timer with no OnTimer still has to implement run. The subclass replaces it.
    let mut run = Asm::new();
    run.ret(0, "void");
    cls.method(MethodModel::new(
        "run",
        "V",
        Vec::new(),
        ACC_PUBLIC,
        Some(run),
        1,
        1,
        1,
    ));
    dex.add(cls);
}

/// A thumb with no 9-patch inset, so the bar reaches the view's right edge.
/// Loaded only from `hugScroll` when the device is API 29 or newer.
fn emit_scroll_thumb(dex: &mut crate::dex::DexFile, package: &str) {
    let desc = class_desc(package, "DroideBar29");
    let mut cls = ClassModel::new(&desc, "Ljava/lang/Object;", ACC_PUBLIC, "DroideBar29");
    let mut thumb = Asm::new();
    // v5 is the view.
    thumb.new_instance(0, "Landroid/graphics/drawable/GradientDrawable;");
    thumb.invoke(
        "direct",
        method("Landroid/graphics/drawable/GradientDrawable;", "<init>", "V", &[]),
        &[0],
    );
    thumb.const_(1, 0x996E_665Cu32 as i32 as i64);
    thumb.invoke(
        "virtual",
        method(
            "Landroid/graphics/drawable/GradientDrawable;",
            "setColor",
            "V",
            &["I"],
        ),
        &[0, 1],
    );
    thumb.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setVerticalScrollbarThumbDrawable",
            "V",
            &["Landroid/graphics/drawable/Drawable;"],
        ),
        &[5, 0],
    );
    thumb.ret(0, "void");
    cls.method(MethodModel::new(
        "thumb",
        "V",
        vec!["Landroid/view/View;".into()],
        ACC_PUBLIC | ACC_STATIC,
        Some(thumb),
        6,
        1,
        2,
    ));
    dex.add(cls);
}

fn emit_ring_rect(draw: &mut Asm, pad: i64) {
    draw.invoke(
        "virtual",
        method("Landroid/graphics/drawable/Drawable;", "getBounds", "Landroid/graphics/Rect;", &[]),
        &[6],
    );
    draw.move_result(1, true);
    draw.iget(2, 1, field("Landroid/graphics/Rect;", "left", "I"));
    draw.iget(3, 1, field("Landroid/graphics/Rect;", "top", "I"));
    draw.iget(4, 1, field("Landroid/graphics/Rect;", "right", "I"));
    draw.iget(5, 1, field("Landroid/graphics/Rect;", "bottom", "I"));
    draw.const_(1, pad);
    draw.binop(0x90, 2, 2, 1, "add-int");
    draw.binop(0x90, 3, 3, 1, "add-int");
    draw.binop(0x91, 4, 4, 1, "sub-int");
    draw.binop(0x91, 5, 5, 1, "sub-int");
    draw.new_instance(1, "Landroid/graphics/Rect;");
    draw.invoke(
        "direct",
        method("Landroid/graphics/Rect;", "<init>", "V", &["I", "I", "I", "I"]),
        &[1, 2, 3, 4, 5],
    );
    draw.invoke(
        "virtual",
        method(
            "Landroid/graphics/Canvas;",
            "drawRect",
            "V",
            &["Landroid/graphics/Rect;", "Landroid/graphics/Paint;"],
        ),
        &[7, 1, 0],
    );
}

/// A vertical mark in the scope gutter, and the selection ring. `v6` is this, `v7` the canvas.
fn emit_rail_mark(dex: &mut crate::dex::DexFile, package: &str) {
    let desc = class_desc(package, "DroideRail");
    let mut cls = ClassModel::new(&desc, "Landroid/graphics/drawable/Drawable;", ACC_PUBLIC, "DroideRail");
    cls.field("paint", "Landroid/graphics/Paint;", ACC_PUBLIC);
    cls.field("ring", "I", ACC_PUBLIC);
    let mut init = Asm::new();
    init.invoke(
        "direct",
        method("Landroid/graphics/drawable/Drawable;", "<init>", "V", &[]),
        &[2],
    );
    init.new_instance(0, "Landroid/graphics/Paint;");
    init.invoke(
        "direct",
        method("Landroid/graphics/Paint;", "<init>", "V", &[]),
        &[0],
    );
    init.invoke(
        "virtual",
        method("Landroid/graphics/Paint;", "setColor", "V", &["I"]),
        &[0, 3],
    );
    init.iput_obj(0, 2, field(&desc, "paint", "Landroid/graphics/Paint;"));
    init.iput(4, 2, field(&desc, "ring", "I"));
    init.ret(0, "void");
    cls.method(MethodModel::new(
        "<init>",
        "V",
        vec!["I".into(), "I".into()],
        ACC_PUBLIC | ACC_CONSTRUCTOR,
        Some(init),
        5,
        3,
        2,
    ));
    let mut draw = Asm::new();
    // A 12px mark on the row's right edge. Open rows stack into one span.
    draw.iget_obj(0, 6, field(&desc, "paint", "Landroid/graphics/Paint;"));
    draw.invoke(
        "virtual",
        method("Landroid/graphics/drawable/Drawable;", "getBounds", "Landroid/graphics/Rect;", &[]),
        &[6],
    );
    draw.move_result(1, true);
    draw.iget(2, 1, field("Landroid/graphics/Rect;", "right", "I"));
    draw.iget(3, 1, field("Landroid/graphics/Rect;", "top", "I"));
    draw.iget(4, 1, field("Landroid/graphics/Rect;", "bottom", "I"));
    draw.const_(5, 18);
    draw.binop(0x91, 5, 2, 5, "sub-int");
    draw.const_(1, 6);
    draw.binop(0x91, 2, 2, 1, "sub-int");
    draw.new_instance(1, "Landroid/graphics/Rect;");
    draw.invoke(
        "direct",
        method("Landroid/graphics/Rect;", "<init>", "V", &["I", "I", "I", "I"]),
        &[1, 5, 3, 2, 4],
    );
    draw.invoke(
        "virtual",
        method(
            "Landroid/graphics/Canvas;",
            "drawRect",
            "V",
            &["Landroid/graphics/Rect;", "Landroid/graphics/Paint;"],
        ),
        &[7, 1, 0],
    );
    draw.iget(5, 6, field(&desc, "ring", "I"));
    draw.if_zero("eqz", 5, "rail_done");
    draw.sget_object(
        5,
        field(
            "Landroid/graphics/Paint$Style;",
            "STROKE",
            "Landroid/graphics/Paint$Style;",
        ),
    );
    draw.invoke(
        "virtual",
        method(
            "Landroid/graphics/Paint;",
            "setStyle",
            "V",
            &["Landroid/graphics/Paint$Style;"],
        ),
        &[0, 5],
    );
    draw.const_(5, 2);
    draw.unop(0x82, 5, 5, "int-to-float");
    draw.invoke(
        "virtual",
        method("Landroid/graphics/Paint;", "setStrokeWidth", "V", &["F"]),
        &[0, 5],
    );
    draw.const_(5, argb(0x00FF_FFFF));
    draw.invoke(
        "virtual",
        method("Landroid/graphics/Paint;", "setColor", "V", &["I"]),
        &[0, 5],
    );
    emit_ring_rect(&mut draw, 1);
    draw.const_(5, argb(0x0011_1111));
    draw.invoke(
        "virtual",
        method("Landroid/graphics/Paint;", "setColor", "V", &["I"]),
        &[0, 5],
    );
    emit_ring_rect(&mut draw, 3);
    draw.label("rail_done");
    draw.ret(0, "void");
    cls.method(MethodModel::new(
        "draw",
        "V",
        vec!["Landroid/graphics/Canvas;".into()],
        ACC_PUBLIC,
        Some(draw),
        8,
        2,
        5,
    ));
    for (name, params) in [
        ("setAlpha", vec!["I".to_string()]),
        ("setColorFilter", vec!["Landroid/graphics/ColorFilter;".to_string()]),
    ] {
        let mut empty = Asm::new();
        empty.ret(0, "void");
        cls.method(MethodModel::new(
            name,
            "V",
            params,
            ACC_PUBLIC,
            Some(empty),
            3,
            2,
            1,
        ));
    }
    let mut opacity = Asm::new();
    opacity.const_(0, -3);
    opacity.ret(0, "int");
    cls.method(MethodModel::new(
        "getOpacity",
        "I",
        Vec::new(),
        ACC_PUBLIC,
        Some(opacity),
        2,
        1,
        1,
    ));
    dex.add(cls);
}

/// `v6` lines, `v7` name. 1 when a whole line matches.
fn emit_line_has(asm: &mut Asm) {
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[6],
    );
    asm.move_result(0, false);
    asm.if_zero("eqz", 0, "has_empty");
    asm.const_string(0, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[6, 0],
    );
    asm.move_result(0, true);
    asm.array_length(1, 0);
    int_zero(asm, 2);
    int_zero(asm, 4);
    asm.const_string(3, "");
    asm.label("has_loop");
    asm.if_cmp("ge", 2, 1, "has_no");
    asm.aget_object(3, 0, 2);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "equals", "Z", &["Ljava/lang/Object;"]),
        &[3, 7],
    );
    asm.move_result(4, false);
    asm.if_zero("nez", 4, "has_yes");
    asm.const_(4, 1);
    asm.binop(0x90, 2, 2, 4, "add-int");
    asm.const_string(3, "");
    asm.goto("has_loop");
    asm.label("has_yes");
    asm.const_(0, 1);
    asm.ret(0, "int");
    asm.label("has_no");
    int_zero(asm, 0);
    asm.ret(0, "int");
    asm.label("has_empty");
    int_zero(asm, 0);
    asm.ret(0, "int");
}

/// `v6` list, `v7` picked titles, one name a line.
fn emit_paint_picked(asm: &mut Asm, rows: &str) {
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/ListView;",
            "getAdapter",
            "Landroid/widget/ListAdapter;",
            &[],
        ),
        &[6],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, "pick_no");
    asm.invoke(
        "virtual",
        method("Ljava/lang/Object;", "getClass", "Ljava/lang/Class;", &[]),
        &[0],
    );
    asm.move_result(1, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Class;", "getName", "Ljava/lang/String;", &[]),
        &[1],
    );
    asm.move_result(1, true);
    asm.const_string(2, "DroideRows");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[1, 2],
    );
    asm.move_result(3, false);
    asm.if_zero("eqz", 3, "pick_no");
    asm.check_cast(0, rows);
    asm.iput_obj(7, 0, field(rows, "picked", "Ljava/lang/String;"));
    asm.invoke(
        "virtual",
        method("Landroid/widget/BaseAdapter;", "notifyDataSetChanged", "V", &[]),
        &[0],
    );
    asm.ret(0, "void");
    asm.label("pick_no");
    asm.ret(0, "void");
}

/// `v9` row, `v10` adapter, `v11` position. An open period paints a rail mark on this row.
fn emit_paint_span(asm: &mut Asm, rows: &str, rail: &str) {
    asm.iget_obj(0, 10, field(rows, "details", "[Ljava/lang/String;"));
    asm.if_zero("eqz", 0, "span_none");
    asm.array_length(1, 0);
    asm.const_(2, 0);
    asm.const_(3, 0);
    asm.const_(5, 0);
    asm.const_string(4, "");
    asm.label("span_loop");
    asm.if_cmp("gt", 3, 11, "span_after");
    asm.if_cmp("ge", 3, 1, "span_after");
    asm.aget_object(4, 0, 3);
    asm.if_zero("eqz", 4, "span_bump");
    asm.const_string(5, "PeriodEnd");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[4, 5],
    );
    asm.move_result(5, false);
    asm.if_zero("nez", 5, "span_close");
    asm.const_string(5, "Period done");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[4, 5],
    );
    asm.move_result(5, false);
    asm.if_zero("nez", 5, "span_open");
    asm.const_string(5, "Period");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[4, 5],
    );
    asm.move_result(5, false);
    asm.if_zero("eqz", 5, "span_bump");
    asm.label("span_open");
    asm.const_(5, 1);
    asm.binop(0x90, 2, 2, 5, "add-int");
    asm.goto("span_bump");
    asm.label("span_close");
    asm.const_(5, 0);
    asm.if_cmp("le", 2, 5, "span_bump");
    asm.const_(5, 1);
    asm.binop(0x91, 2, 2, 5, "sub-int");
    asm.label("span_bump");
    asm.const_(5, 1);
    asm.binop(0x90, 3, 3, 5, "add-int");
    asm.const_string(4, "");
    asm.goto("span_loop");
    asm.label("span_after");
    int_zero(asm, 8);
    int_zero(asm, 5);
    asm.invoke(
        "virtual",
        method("Landroid/widget/ArrayAdapter;", "getItem", "Ljava/lang/Object;", &["I"]),
        &[10, 11],
    );
    asm.move_result(4, true);
    asm.if_zero("eqz", 4, "span_norow");
    asm.invoke(
        "virtual",
        method("Ljava/lang/Object;", "toString", "Ljava/lang/String;", &[]),
        &[4],
    );
    asm.move_result(4, true);
    asm.iget_obj(5, 10, field(rows, "picked", "Ljava/lang/String;"));
    asm.invoke(
        "static",
        method(
            &rail.replace("DroideRail", "DroideRt"),
            "lineHas",
            "I",
            &["Ljava/lang/String;", "Ljava/lang/String;"],
        ),
        &[5, 4],
    );
    asm.move_result(8, false);
    int_zero(asm, 5);
    asm.label("span_norow");
    int_zero(asm, 5);
    asm.if_cmp("le", 2, 5, "span_nocolor");
    asm.iget(6, 10, field(rows, "ink", "I"));
    asm.goto("span_mark");
    asm.label("span_nocolor");
    int_zero(asm, 6);
    int_zero(asm, 5);
    asm.if_cmp("eq", 8, 5, "span_clear");
    asm.label("span_mark");
    asm.new_instance(0, rail);
    asm.invoke(
        "direct",
        method(rail, "<init>", "V", &["I", "I"]),
        &[0, 6, 8],
    );
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setForeground", "V", &["Landroid/graphics/drawable/Drawable;"]),
        &[9, 0],
    );
    asm.ret(0, "void");
    asm.label("span_clear");
    asm.const_(0, 0);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setForeground", "V", &["Landroid/graphics/drawable/Drawable;"]),
        &[9, 0],
    );
    asm.ret(0, "void");
    asm.label("span_none");
    asm.const_(0, 0);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setForeground", "V", &["Landroid/graphics/drawable/Drawable;"]),
        &[9, 0],
    );
    asm.ret(0, "void");
}

/// Paints a list color only across its design width, so the gutter stays clear.
fn emit_inset_background(dex: &mut crate::dex::DexFile, package: &str) {
    let desc = class_desc(package, "DroideInset");
    let mut cls = ClassModel::new(
        &desc,
        "Landroid/graphics/drawable/ColorDrawable;",
        ACC_PUBLIC,
        "DroideInset",
    );
    cls.field("pad", "I", ACC_PUBLIC);
    let mut init = Asm::new();
    init.invoke(
        "direct",
        method(
            "Landroid/graphics/drawable/ColorDrawable;",
            "<init>",
            "V",
            &["I"],
        ),
        &[0, 1],
    );
    init.iput(2, 0, field(&desc, "pad", "I"));
    init.ret(0, "void");
    cls.method(MethodModel::new(
        "<init>",
        "V",
        vec!["I".into(), "I".into()],
        ACC_PUBLIC | ACC_CONSTRUCTOR,
        Some(init),
        3,
        3,
        2,
    ));
    let mut draw = Asm::new();
    // v6 this, v7 canvas.
    draw.invoke(
        "virtual",
        method(
            "Landroid/graphics/drawable/Drawable;",
            "getBounds",
            "Landroid/graphics/Rect;",
            &[],
        ),
        &[6],
    );
    draw.move_result(0, true);
    draw.iget(1, 0, field("Landroid/graphics/Rect;", "left", "I"));
    draw.iget(2, 0, field("Landroid/graphics/Rect;", "top", "I"));
    draw.iget(3, 0, field("Landroid/graphics/Rect;", "right", "I"));
    draw.iget(4, 0, field("Landroid/graphics/Rect;", "bottom", "I"));
    draw.iget(5, 6, field(&desc, "pad", "I"));
    draw.binop(0x91, 3, 3, 5, "sub-int");
    draw.invoke("virtual", method("Landroid/graphics/Canvas;", "save", "I", &[]), &[7]);
    draw.move_result(0, false);
    draw.invoke(
        "virtual",
        method("Landroid/graphics/Canvas;", "clipRect", "Z", &["I", "I", "I", "I"]),
        &[7, 1, 2, 3, 4],
    );
    draw.move_result(0, false);
    draw.invoke(
        "super",
        method(
            "Landroid/graphics/drawable/ColorDrawable;",
            "draw",
            "V",
            &["Landroid/graphics/Canvas;"],
        ),
        &[6, 7],
    );
    draw.invoke(
        "virtual",
        method("Landroid/graphics/Canvas;", "restore", "V", &[]),
        &[7],
    );
    draw.ret(0, "void");
    cls.method(MethodModel::new(
        "draw",
        "V",
        vec!["Landroid/graphics/Canvas;".into()],
        ACC_PUBLIC,
        Some(draw),
        8,
        2,
        5,
    ));
    dex.add(cls);
}

/// `v14` is the view and `v15` its design width in px. A negative width only restyles the bar.
fn emit_hug_scroll(asm: &mut Asm, bar: &str) {
    // OUTSIDE_OVERLAY. Padding stays inside; the bar is on the view's outer edge.
    asm.const_(0, 0x0200_0000);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setScrollBarStyle", "V", &["I"]),
        &[14, 0],
    );
    asm.const_(1, 0);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setScrollbarFadingEnabled", "V", &["Z"]),
        &[14, 1],
    );
    asm.sget(0, field("Landroid/os/Build$VERSION;", "SDK_INT", "I"));
    asm.const_(1, 29);
    asm.if_cmp("lt", 0, 1, "wide");
    asm.invoke(
        "static",
        method(bar, "thumb", "V", &["Landroid/view/View;"]),
        &[14],
    );
    asm.label("wide");
    asm.const_(0, 0);
    asm.if_cmp("lt", 15, 0, "neg");
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "getLayoutParams",
            "Landroid/view/ViewGroup$LayoutParams;",
            &[],
        ),
        &[14],
    );
    asm.move_result(2, true);
    asm.if_zero("eqz", 2, "noparams");
    asm.check_cast(2, "Landroid/view/ViewGroup$MarginLayoutParams;");
    asm.iget(
        3,
        2,
        field(
            "Landroid/view/ViewGroup$MarginLayoutParams;",
            "leftMargin",
            "I",
        ),
    );
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "getContext", "Landroid/content/Context;", &[]),
        &[14],
    );
    asm.move_result(4, true);
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/Context;",
            "getResources",
            "Landroid/content/res/Resources;",
            &[],
        ),
        &[4],
    );
    asm.move_result(4, true);
    asm.invoke(
        "virtual",
        method(
            "Landroid/content/res/Resources;",
            "getDisplayMetrics",
            "Landroid/util/DisplayMetrics;",
            &[],
        ),
        &[4],
    );
    asm.move_result(4, true);
    asm.iget(5, 4, field("Landroid/util/DisplayMetrics;", "widthPixels", "I"));
    asm.binop(0x91, 5, 5, 3, "sub-int");
    asm.binop(0x91, 5, 5, 15, "sub-int");
    asm.const_(0, 0);
    asm.if_cmp("le", 5, 0, "noextra");
    asm.binop(0x90, 6, 15, 5, "add-int");
    asm.iput(
        6,
        2,
        field("Landroid/view/ViewGroup$LayoutParams;", "width", "I"),
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setLayoutParams",
            "V",
            &["Landroid/view/ViewGroup$LayoutParams;"],
        ),
        &[14, 2],
    );
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "getPaddingLeft", "I", &[]),
        &[14],
    );
    asm.move_result(6, false);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "getPaddingTop", "I", &[]),
        &[14],
    );
    asm.move_result(7, false);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "getPaddingBottom", "I", &[]),
        &[14],
    );
    asm.move_result(8, false);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setPadding", "V", &["I", "I", "I", "I"]),
        &[14, 6, 7, 5, 8],
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "getBackground",
            "Landroid/graphics/drawable/Drawable;",
            &[],
        ),
        &[14],
    );
    asm.move_result(6, true);
    asm.if_zero("eqz", 6, "nobg");
    asm.check_cast(6, "Landroid/graphics/drawable/ColorDrawable;");
    asm.invoke(
        "virtual",
        method("Landroid/graphics/drawable/ColorDrawable;", "getColor", "I", &[]),
        &[6],
    );
    asm.move_result(1, false);
    let inset = bar.replace("DroideBar29", "DroideInset");
    asm.new_instance(0, &inset);
    asm.invoke(
        "direct",
        method(&inset, "<init>", "V", &["I", "I"]),
        &[0, 1, 5],
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setBackground",
            "V",
            &["Landroid/graphics/drawable/Drawable;"],
        ),
        &[14, 0],
    );
    asm.ret(0, "void");
    asm.label("neg");
    asm.ret(0, "void");
    asm.label("noparams");
    asm.ret(0, "void");
    asm.label("noextra");
    asm.ret(0, "void");
    asm.label("nobg");
    asm.ret(0, "void");
}

/// Keeps a drag on the list instead of letting the form's ScrollView take it.
fn emit_list_scroll(dex: &mut crate::dex::DexFile, package: &str) {
    let desc = class_desc(package, "DroideListScroll");
    let mut cls = ClassModel::new(&desc, "Ljava/lang/Object;", ACC_PUBLIC, "DroideListScroll");
    cls.interfaces()
        .push("Landroid/view/View$OnTouchListener;".into());
    let mut init = Asm::new();
    init.invoke(
        "direct",
        method("Ljava/lang/Object;", "<init>", "V", &[]),
        &[0],
    );
    init.ret(0, "void");
    cls.method(MethodModel::new(
        "<init>",
        "V",
        Vec::new(),
        ACC_PUBLIC | ACC_CONSTRUCTOR,
        Some(init),
        1,
        1,
        1,
    ));
    let mut touch = Asm::new();
    // v2 this, v3 the list, v4 the motion event.
    touch.invoke(
        "virtual",
        method("Landroid/view/View;", "getParent", "Landroid/view/ViewParent;", &[]),
        &[3],
    );
    touch.move_result(0, true);
    touch.const_(1, 1);
    touch.invoke(
        "interface",
        method(
            "Landroid/view/ViewParent;",
            "requestDisallowInterceptTouchEvent",
            "V",
            &["Z"],
        ),
        &[0, 1],
    );
    touch.const_(0, 0);
    touch.ret(0, "int");
    cls.method(MethodModel::new(
        "onTouch",
        "Z",
        vec![
            "Landroid/view/View;".into(),
            "Landroid/view/MotionEvent;".into(),
        ],
        ACC_PUBLIC,
        Some(touch),
        5,
        3,
        2,
    ));
    dex.add(cls);
}

/// `v5` row, `v6` detail, `v7` muted colour. A task line gains a box. Done is crossed out.
fn emit_paint_task(asm: &mut Asm) {
    asm.if_zero("eqz", 6, "task_no");
    asm.const_string(0, " done");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[6, 0],
    );
    asm.move_result(1, false);
    asm.if_zero("nez", 1, "task_done");
    asm.const_string(0, "Task");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[6, 0],
    );
    asm.move_result(1, false);
    asm.if_zero("nez", 1, "task_open");
    asm.label("task_no");
    asm.ret(0, "void");
    asm.label("task_done");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[6],
    );
    asm.move_result(1, false);
    asm.const_(2, 5);
    asm.binop(0x91, 1, 1, 2, "sub-int");
    asm.const_(2, 0);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "substring", "Ljava/lang/String;", &["I", "I"]),
        &[6, 2, 1],
    );
    asm.move_result(0, true);
    asm.const_string(1, "\u{2611} ");
    asm.const_(2, 1);
    asm.const_(4, 0);
    asm.goto("task_mark");
    asm.label("task_open");
    asm.move_(0, 6, true);
    asm.const_string(1, "\u{2610} ");
    asm.const_(2, 0);
    asm.const_(4, 0);
    asm.goto("task_mark");
    asm.label("task_mark");
    asm.const_(3, 0x01020015);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "findViewById", "Landroid/view/View;", &["I"]),
        &[5, 3],
    );
    asm.move_result(4, true);
    asm.if_zero("eqz", 4, "task_type_miss");
    asm.check_cast(4, "Landroid/widget/TextView;");
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/TextView;",
            "setText",
            "V",
            &["Ljava/lang/CharSequence;"],
        ),
        &[4, 0],
    );
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setTextColor", "V", &["I"]),
        &[4, 7],
    );
    asm.const_(4, 0);
    asm.goto("task_title");
    asm.label("task_type_miss");
    asm.const_(4, 0);
    asm.label("task_title");
    asm.const_(3, 0x01020014);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "findViewById", "Landroid/view/View;", &["I"]),
        &[5, 3],
    );
    asm.move_result(4, true);
    asm.if_zero("eqz", 4, "task_skip");
    asm.check_cast(4, "Landroid/widget/TextView;");
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "getText", "Ljava/lang/CharSequence;", &[]),
        &[4],
    );
    asm.move_result(3, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Object;", "toString", "Ljava/lang/String;", &[]),
        &[3],
    );
    asm.move_result(3, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "concat", "Ljava/lang/String;", &["Ljava/lang/String;"]),
        &[1, 3],
    );
    asm.move_result(1, true);
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/TextView;",
            "setText",
            "V",
            &["Ljava/lang/CharSequence;"],
        ),
        &[4, 1],
    );
    asm.if_zero("eqz", 2, "task_open_done");
    asm.const_(3, 17);
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setPaintFlags", "V", &["I"]),
        &[4, 3],
    );
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "setTextColor", "V", &["I"]),
        &[4, 7],
    );
    asm.ret(0, "void");
    asm.label("task_open_done");
    asm.ret(0, "void");
    asm.label("task_skip");
    asm.ret(0, "void");
}

/// `v5` list, `v6` which (0 paper, 1 card, 2 ink, 3 muted, 4 size), `v7` value.
fn emit_row_paint(asm: &mut Asm, rows: &str) {
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/ListView;",
            "getAdapter",
            "Landroid/widget/ListAdapter;",
            &[],
        ),
        &[5],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, "none");
    // A one-line list is a plain ArrayAdapter. Leave it alone.
    asm.invoke(
        "virtual",
        method("Ljava/lang/Object;", "getClass", "Ljava/lang/Class;", &[]),
        &[0],
    );
    asm.move_result(1, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Class;", "getName", "Ljava/lang/String;", &[]),
        &[1],
    );
    asm.move_result(1, true);
    asm.const_string(2, "DroideRows");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[1, 2],
    );
    asm.move_result(3, false);
    asm.if_zero("eqz", 3, "plain");
    asm.check_cast(0, rows);
    asm.const_(1, 0);
    asm.if_cmp("eq", 6, 1, "paper");
    asm.const_(1, 1);
    asm.if_cmp("eq", 6, 1, "card");
    asm.const_(1, 2);
    asm.if_cmp("eq", 6, 1, "ink");
    asm.const_(1, 3);
    asm.if_cmp("eq", 6, 1, "muted");
    asm.iput(7, 0, field(rows, "rowSize", "I"));
    asm.goto("fresh");
    asm.label("paper");
    asm.iput(7, 0, field(rows, "paper", "I"));
    asm.goto("fresh");
    asm.label("card");
    asm.iput(7, 0, field(rows, "card", "I"));
    asm.goto("fresh");
    asm.label("ink");
    asm.iput(7, 0, field(rows, "ink", "I"));
    asm.goto("fresh");
    asm.label("muted");
    asm.iput(7, 0, field(rows, "muted", "I"));
    asm.label("fresh");
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/BaseAdapter;",
            "notifyDataSetChanged",
            "V",
            &[],
        ),
        &[0],
    );
    asm.ret(0, "void");
    asm.label("plain");
    asm.ret(0, "void");
    asm.label("none");
    asm.ret(0, "void");
}

/// A negative bottom leaves the vertical edge alone. Otherwise the view sits on the bottom.
fn emit_place(asm: &mut Asm, rt: &str) {
    // registers 8, ins 3. v5 view, v6 left dp, v7 bottom dp.
    asm.if_zero("nez", 5, "place_view");
    asm.ret(0, "void");
    asm.label("place_view");
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "getLayoutParams",
            "Landroid/view/ViewGroup$LayoutParams;",
            &[],
        ),
        &[5],
    );
    asm.move_result(0, true);
    asm.if_zero("nez", 0, "place_box");
    asm.ret(0, "void");
    asm.label("place_box");
    asm.check_cast(0, "Landroid/widget/FrameLayout$LayoutParams;");
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "getContext", "Landroid/content/Context;", &[]),
        &[5],
    );
    asm.move_result(1, true);
    asm.invoke(
        "static",
        method(rt, "dp", "I", &["Landroid/content/Context;", "I"]),
        &[1, 6],
    );
    asm.move_result(2, false);
    asm.iput(
        2,
        0,
        field(
            "Landroid/view/ViewGroup$MarginLayoutParams;",
            "leftMargin",
            "I",
        ),
    );
    int_zero(asm, 3);
    asm.if_cmp("lt", 7, 3, "place_done");
    asm.const_(3, 80);
    asm.iput(
        3,
        0,
        field("Landroid/widget/FrameLayout$LayoutParams;", "gravity", "I"),
    );
    asm.invoke(
        "static",
        method(rt, "dp", "I", &["Landroid/content/Context;", "I"]),
        &[1, 7],
    );
    asm.move_result(2, false);
    asm.iput(
        2,
        0,
        field(
            "Landroid/view/ViewGroup$MarginLayoutParams;",
            "bottomMargin",
            "I",
        ),
    );
    asm.label("place_done");
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setLayoutParams",
            "V",
            &["Landroid/view/ViewGroup$LayoutParams;"],
        ),
        &[5, 0],
    );
    asm.ret(0, "void");
}

/// One Return counts as another line, including a trailing one. The field grows
/// upward from its first height and stops at eight lines, then it scrolls.
const TAG_FLOOR: i64 = 0x7f01_0003;

fn emit_grow(asm: &mut Asm) {
    // registers 12, ins 1. v11 is the view. v1 is the text. v2 is the line count. v3 is the index.
    asm.if_zero("nez", 11, "grow_view");
    asm.ret(0, "void");
    asm.label("grow_view");
    asm.check_cast(11, "Landroid/widget/TextView;");
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/TextView;",
            "getText",
            "Ljava/lang/CharSequence;",
            &[],
        ),
        &[11],
    );
    asm.move_result(1, true);
    asm.if_zero("nez", 1, "grow_words");
    asm.const_string(1, "");
    asm.goto("grow_ready");
    asm.label("grow_words");
    asm.invoke(
        "virtual",
        method("Ljava/lang/Object;", "toString", "Ljava/lang/String;", &[]),
        &[1],
    );
    asm.move_result(1, true);
    asm.label("grow_ready");
    asm.const_(2, 1);
    int_zero(asm, 3);
    int_zero(asm, 0);
    int_zero(asm, 4);
    int_zero(asm, 5);
    asm.label("grow_loop");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[1],
    );
    asm.move_result(4, false);
    asm.if_cmp("ge", 3, 4, "grow_sized");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "charAt", "C", &["I"]),
        &[1, 3],
    );
    asm.move_result(5, false);
    asm.const_(0, 10);
    asm.if_cmp("ne", 5, 0, "grow_next");
    asm.const_(0, 1);
    asm.binop(0x90, 2, 2, 0, "add-int");
    asm.label("grow_next");
    asm.const_(0, 1);
    asm.binop(0x90, 3, 3, 0, "add-int");
    asm.goto("grow_loop");
    asm.label("grow_sized");
    asm.const_(0, 8);
    asm.if_cmp("le", 2, 0, "grow_capped");
    asm.const_(2, 8);
    asm.label("grow_capped");
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "getLineHeight", "I", &[]),
        &[11],
    );
    asm.move_result(4, false);
    asm.const_(0, 1);
    asm.if_cmp("ge", 4, 0, "grow_line");
    asm.const_(4, 1);
    asm.label("grow_line");
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "getCompoundPaddingTop", "I", &[]),
        &[11],
    );
    asm.move_result(5, false);
    asm.invoke(
        "virtual",
        method("Landroid/widget/TextView;", "getCompoundPaddingBottom", "I", &[]),
        &[11],
    );
    asm.move_result(0, false);
    asm.binop(0x90, 5, 5, 0, "add-int");
    asm.binop(0x92, 4, 4, 2, "mul-int");
    asm.binop(0x90, 4, 5, 4, "add-int");
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "getLayoutParams",
            "Landroid/view/ViewGroup$LayoutParams;",
            &[],
        ),
        &[11],
    );
    asm.move_result(6, true);
    asm.if_zero("nez", 6, "grow_box");
    asm.ret(0, "void");
    asm.label("grow_box");
    asm.const_(0, TAG_FLOOR);
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "getTag", "Ljava/lang/Object;", &["I"]),
        &[11, 0],
    );
    asm.move_result(8, true);
    asm.if_zero("nez", 8, "grow_kept");
    asm.iget(7, 6, field("Landroid/view/ViewGroup$LayoutParams;", "height", "I"));
    int_zero(asm, 0);
    asm.if_cmp("gt", 7, 0, "grow_store");
    asm.const_(7, 0);
    asm.binop(0x90, 7, 4, 7, "add-int");
    asm.label("grow_store");
    asm.invoke(
        "static",
        method(
            "Ljava/lang/Integer;",
            "valueOf",
            "Ljava/lang/Integer;",
            &["I"],
        ),
        &[7],
    );
    asm.move_result(8, true);
    asm.const_(0, TAG_FLOOR);
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setTag",
            "V",
            &["I", "Ljava/lang/Object;"],
        ),
        &[11, 0, 8],
    );
    asm.goto("grow_floor");
    asm.label("grow_kept");
    asm.check_cast(8, "Ljava/lang/Integer;");
    asm.invoke(
        "virtual",
        method("Ljava/lang/Integer;", "intValue", "I", &[]),
        &[8],
    );
    asm.move_result(7, false);
    asm.label("grow_floor");
    asm.if_cmp("ge", 4, 7, "grow_set");
    asm.const_(4, 0);
    asm.binop(0x90, 4, 7, 4, "add-int");
    asm.label("grow_set");
    asm.iput(4, 6, field("Landroid/view/ViewGroup$LayoutParams;", "height", "I"));
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setLayoutParams",
            "V",
            &["Landroid/view/ViewGroup$LayoutParams;"],
        ),
        &[11, 6],
    );
    asm.ret(0, "void");
}

/// `v5` view, `v6` context, `v7` height in dp.
fn emit_set_dp_height(asm: &mut Asm, rt: &str) {
    asm.invoke(
        "static",
        method(rt, "dp", "I", &["Landroid/content/Context;", "I"]),
        &[6, 7],
    );
    asm.move_result(0, false);
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "getLayoutParams",
            "Landroid/view/ViewGroup$LayoutParams;",
            &[],
        ),
        &[5],
    );
    asm.move_result(1, true);
    asm.if_zero("eqz", 1, "skip");
    asm.iput(0, 1, field("Landroid/view/ViewGroup$LayoutParams;", "height", "I"));
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setLayoutParams",
            "V",
            &["Landroid/view/ViewGroup$LayoutParams;"],
        ),
        &[5, 1],
    );
    asm.ret(0, "void");
    asm.label("skip");
    asm.ret(0, "void");
}

/// `v6` view, `v7` colour. Keeps an oval plaque and a clipped list gutter.
fn emit_recolor(asm: &mut Asm) {
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "getBackground",
            "Landroid/graphics/drawable/Drawable;",
            &[],
        ),
        &[6],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, "plain_null");
    asm.invoke(
        "virtual",
        method("Ljava/lang/Object;", "getClass", "Ljava/lang/Class;", &[]),
        &[0],
    );
    asm.move_result(1, true);
    asm.invoke(
        "virtual",
        method("Ljava/lang/Class;", "getName", "Ljava/lang/String;", &[]),
        &[1],
    );
    asm.move_result(1, true);
    asm.const_string(2, "GradientDrawable");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[1, 2],
    );
    asm.move_result(3, false);
    asm.if_zero("nez", 3, "grad");
    asm.const_string(2, "DroideInset");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[1, 2],
    );
    asm.move_result(3, false);
    asm.if_zero("nez", 3, "ink");
    asm.const_string(2, "ColorDrawable");
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "endsWith", "Z", &["Ljava/lang/String;"]),
        &[1, 2],
    );
    asm.move_result(3, false);
    asm.if_zero("eqz", 3, "plain_other");
    asm.label("ink");
    asm.check_cast(0, "Landroid/graphics/drawable/ColorDrawable;");
    asm.invoke(
        "virtual",
        method(
            "Landroid/graphics/drawable/ColorDrawable;",
            "setColor",
            "V",
            &["I"],
        ),
        &[0, 7],
    );
    asm.ret(0, "void");
    asm.label("grad");
    asm.check_cast(0, "Landroid/graphics/drawable/GradientDrawable;");
    asm.invoke(
        "virtual",
        method(
            "Landroid/graphics/drawable/GradientDrawable;",
            "setColor",
            "V",
            &["I"],
        ),
        &[0, 7],
    );
    asm.ret(0, "void");
    asm.label("plain_null");
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setBackgroundColor", "V", &["I"]),
        &[6, 7],
    );
    asm.ret(0, "void");
    asm.label("plain_other");
    asm.invoke(
        "virtual",
        method("Landroid/view/View;", "setBackgroundColor", "V", &["I"]),
        &[6, 7],
    );
    asm.ret(0, "void");
}

/// `v6` activity, `v7` colour. Paints the page behind the widgets.
fn emit_page(asm: &mut Asm, rt: &str) {
    asm.const_(0, 0x0102_0002);
    asm.invoke(
        "virtual",
        method(
            "Landroid/app/Activity;",
            "findViewById",
            "Landroid/view/View;",
            &["I"],
        ),
        &[6, 0],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, "no_content");
    asm.check_cast(0, "Landroid/view/ViewGroup;");
    asm.const_(1, 0);
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/ViewGroup;",
            "getChildAt",
            "Landroid/view/View;",
            &["I"],
        ),
        &[0, 1],
    );
    asm.move_result(1, true);
    asm.if_zero("eqz", 1, "no_scroll");
    asm.invoke(
        "static",
        method(rt, "recolor", "V", &["Landroid/view/View;", "I"]),
        &[1, 7],
    );
    asm.check_cast(1, "Landroid/view/ViewGroup;");
    asm.const_(0, 0);
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/ViewGroup;",
            "getChildAt",
            "Landroid/view/View;",
            &["I"],
        ),
        &[1, 0],
    );
    asm.move_result(0, true);
    asm.if_zero("eqz", 0, "no_frame");
    asm.invoke(
        "static",
        method(rt, "recolor", "V", &["Landroid/view/View;", "I"]),
        &[0, 7],
    );
    asm.ret(0, "void");
    asm.label("no_content");
    asm.ret(0, "void");
    asm.label("no_scroll");
    asm.ret(0, "void");
    asm.label("no_frame");
    asm.ret(0, "void");
}

fn emit_rows_class(dex: &mut crate::dex::DexFile, package: &str) {
    let desc = class_desc(package, "DroideRows");
    let mut cls = ClassModel::new(&desc, "Landroid/widget/ArrayAdapter;", ACC_PUBLIC, "DroideRows");
    cls.field("details", "[Ljava/lang/String;", ACC_PUBLIC);
    cls.field("paper", "I", ACC_PUBLIC);
    cls.field("card", "I", ACC_PUBLIC);
    cls.field("ink", "I", ACC_PUBLIC);
    cls.field("muted", "I", ACC_PUBLIC);
    cls.field("rowSize", "I", ACC_PUBLIC);
    cls.field("picked", "Ljava/lang/String;", ACC_PUBLIC);
    let mut init = Asm::new();
    emit_rows_init(&mut init, &desc);
    cls.method(MethodModel::new(
        "<init>",
        "V",
        vec![
            "Landroid/content/Context;".into(),
            "I".into(),
            "[Ljava/lang/Object;".into(),
            "[Ljava/lang/String;".into(),
        ],
        ACC_PUBLIC | ACC_CONSTRUCTOR,
        Some(init),
        6,
        5,
        5,
    ));
    let mut view = Asm::new();
    emit_rows_view(&mut view, &desc);
    cls.method(MethodModel::new(
        "getView",
        "Landroid/view/View;",
        vec![
            "I".into(),
            "Landroid/view/View;".into(),
            "Landroid/view/ViewGroup;".into(),
        ],
        ACC_PUBLIC,
        Some(view),
        8,
        4,
        5,
    ));
    dex.add(cls);
}

/// Same rows as `setLines`. Spinner extends AbsSpinner, so the adapter call is
/// `setAdapter(SpinnerAdapter)`, not the AbsListView method List uses.
fn emit_set_spinner(asm: &mut Asm) {
    // v5 spinner, v6 context, v7 text. registers 8, ins 3.
    // TAG_ITEMS keeps the text so Items can be read back. A spinner has no details.
    asm.const_(0, TAG_ITEMS);
    asm.invoke(
        "virtual",
        method(
            "Landroid/view/View;",
            "setTag",
            "V",
            &["I", "Ljava/lang/Object;"],
        ),
        &[5, 0, 7],
    );
    asm.invoke(
        "virtual",
        method("Ljava/lang/String;", "length", "I", &[]),
        &[7],
    );
    asm.move_result(0, false);
    asm.if_zero("nez", 0, "lines_have");
    asm.const_(3, 0);
    asm.new_array(0, 3, "[Ljava/lang/String;");
    asm.goto("lines_ready");
    asm.label("lines_have");
    asm.const_string(1, "\n");
    asm.invoke(
        "virtual",
        method(
            "Ljava/lang/String;",
            "split",
            "[Ljava/lang/String;",
            &["Ljava/lang/String;"],
        ),
        &[7, 1],
    );
    asm.move_result(0, true);
    asm.label("lines_ready");
    asm.check_cast(0, "[Ljava/lang/Object;");
    asm.const_(2, 0x01090003);
    asm.new_instance(1, "Landroid/widget/ArrayAdapter;");
    asm.invoke(
        "direct",
        method(
            "Landroid/widget/ArrayAdapter;",
            "<init>",
            "V",
            &[
                "Landroid/content/Context;",
                "I",
                "[Ljava/lang/Object;",
            ],
        ),
        &[1, 6, 2, 0],
    );
    asm.invoke(
        "virtual",
        method(
            "Landroid/widget/AbsSpinner;",
            "setAdapter",
            "V",
            &["Landroid/widget/SpinnerAdapter;"],
        ),
        &[5, 1],
    );
    asm.ret(0, "void");
}

fn emit_web_settings(asm: &mut Asm, web: u16, tmp: u16, flag: u16) {
    asm.invoke(
        "virtual",
        method(
            "Landroid/webkit/WebView;",
            "getSettings",
            "Landroid/webkit/WebSettings;",
            &[],
        ),
        &[web],
    );
    asm.move_result(tmp, true);
    asm.const_(flag, 1);
    asm.invoke(
        "virtual",
        method(
            "Landroid/webkit/WebSettings;",
            "setJavaScriptEnabled",
            "V",
            &["Z"],
        ),
        &[tmp, flag],
    );
}

/// HTML wins when both are set. An empty value is skipped so it does not blank the view.
/// `#`, `%`, and `\` are unsafe in loadData because it treats them as URL encoding.
fn emit_web_load(asm: &mut Asm, props: Option<&Map<String, Value>>) {
    let html = prop_text(props, "Html");
    let url = prop_text(props, "Url");
    if html.is_empty() && url.is_empty() {
        return;
    }
    emit_web_settings(asm, 2, 3, 4);
    if !html.is_empty() {
        asm.const_string(3, &html);
        asm.const_string(4, "text/html");
        asm.const_string(5, "utf-8");
        asm.invoke(
            "virtual",
            method(
                "Landroid/webkit/WebView;",
                "loadData",
                "V",
                &[
                    "Ljava/lang/String;",
                    "Ljava/lang/String;",
                    "Ljava/lang/String;",
                ],
            ),
            &[2, 3, 4, 5],
        );
    } else {
        asm.const_string(3, &url);
        asm.invoke(
            "virtual",
            method(
                "Landroid/webkit/WebView;",
                "loadUrl",
                "V",
                &["Ljava/lang/String;"],
            ),
            &[2, 3],
        );
    }
}

fn prop_text(props: Option<&Map<String, Value>>, name: &str) -> String {
    props
        .and_then(|map| map.get(name))
        .map(py_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn class_desc(package: &str, name: &str) -> String {
    format!("L{}/{name};", package.replace('.', "/"))
}

fn type_desc(ty: &str) -> Result<String, CompileError> {
    match ty {
        "Integer" => Ok("I".into()),
        "Boolean" => Ok("Z".into()),
        "String" => Ok("Ljava/lang/String;".into()),
        "View" => Ok("Landroid/view/View;".into()),
        other => Err(CompileError::bare(format!("unknown type {other}"))),
    }
}

fn canon_type(name: &str) -> String {
    match name.to_lowercase().as_str() {
        "integer" => "Integer".into(),
        "string" => "String".into(),
        "boolean" => "Boolean".into(),
        "view" => "View".into(),
        _ => name.to_string(),
    }
}

fn is_obj(ty: &str) -> bool {
    ty == "String" || ty == "View"
}

fn int_bin(op: &str) -> Option<(u8, &'static str)> {
    match op {
        "+" => Some((0x90, "add-int")),
        "-" => Some((0x91, "sub-int")),
        "*" => Some((0x92, "mul-int")),
        "/" | "div" => Some((0x93, "div-int")),
        "mod" => Some((0x94, "rem-int")),
        _ => None,
    }
}

fn cmp_kind(op: &str) -> Option<&'static str> {
    match op {
        "=" => Some("eq"),
        "<>" => Some("ne"),
        "<" => Some("lt"),
        ">" => Some("gt"),
        "<=" => Some("le"),
        ">=" => Some("ge"),
        _ => None,
    }
}

fn method(owner: &str, name: &str, ret: &str, params: &[&str]) -> MethodRef {
    MethodRef {
        owner: owner.to_string(),
        name: name.to_string(),
        ret: ret.to_string(),
        params: params.iter().map(|item| (*item).to_string()).collect(),
    }
}

fn field(owner: &str, name: &str, ty: &str) -> FieldRef {
    FieldRef {
        owner: owner.to_string(),
        name: name.to_string(),
        ty: ty.to_string(),
    }
}

fn dex_method(spec: &WMethod) -> MethodRef {
    MethodRef {
        owner: spec.owner.to_string(),
        name: spec.name.to_string(),
        ret: spec.ret.to_string(),
        params: spec.params.iter().map(|item| (*item).to_string()).collect(),
    }
}

fn setter_ref(name: &str) -> Result<&'static WMethod, CompileError> {
    setter(name).ok_or_else(|| CompileError::bare(format!("unknown property {name}")))
}

fn getter_ref(name: &str) -> Result<&'static WMethod, CompileError> {
    getter(name).ok_or_else(|| CompileError::bare(format!("unknown property {name}")))
}

fn spec_exact(name: &str) -> Result<&'static WidgetSpec, CompileError> {
    WIDGETS
        .iter()
        .find(|spec| spec.name == name)
        .ok_or_else(|| CompileError::bare(format!("unknown component type {name}")))
}

fn components(form: &Value) -> &[Value] {
    form.get("components")
        .and_then(|v| v.as_array())
        .map(|items| items.as_slice())
        .unwrap_or(&[])
}

fn event_of<'a>(obj: &'a Value, name: &str) -> Option<&'a str> {
    let events = obj.get("events")?;
    if !events.is_object() {
        return None;
    }
    let text = events.get(name)?.as_str()?;
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn comp_parent(comp: &Value) -> &str {
    comp.get("parent")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
}

fn direction_horizontal(comp: &Value) -> bool {
    comp.get("props")
        .and_then(|props| props.get("Direction"))
        .and_then(|v| v.as_str())
        .is_some_and(|text| text.trim().eq_ignore_ascii_case("Horizontal"))
}

fn comp_str<'a>(comp: &'a Value, key: &str) -> Result<&'a str, CompileError> {
    comp.get(key)
        .and_then(|v| v.as_str())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| CompileError::bare(format!("component has no {key}")))
}

fn background_color(form: &Value) -> Result<i64, CompileError> {
    let color = match form.get("background") {
        Some(value) if truthy(value) => parse_color(value).map_err(CompileError::bare)?,
        _ => parse_color(&Value::String("#F7F8FA".into())).map_err(CompileError::bare)?,
    };
    Ok(color as i64)
}

fn int_or(obj: &Value, key: &str, default: i64) -> Result<i64, CompileError> {
    match obj.get(key) {
        Some(value) if truthy(value) => json_int(value),
        _ => Ok(default),
    }
}

fn comp_int(comp: &Value, key: &str) -> Result<i64, CompileError> {
    match comp.get(key) {
        Some(value) => json_int(value),
        None => Ok(0),
    }
}

fn prop_or(
    props: Option<&Map<String, Value>>,
    defaults: &Value,
    key: &str,
) -> Result<i64, CompileError> {
    if let Some(value) = props.and_then(|map| map.get(key)) {
        return json_int(value);
    }
    match defaults.get(key) {
        Some(value) => json_int(value),
        None => Err(CompileError::bare(format!("missing {key}"))),
    }
}

fn json_int(value: &Value) -> Result<i64, CompileError> {
    match value {
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(value)
            } else if let Some(value) = number.as_u64() {
                i64::try_from(value).map_err(|_| CompileError::bare("integer overflow"))
            } else if let Some(value) = number.as_f64() {
                Ok(value as i64)
            } else {
                Err(CompileError::bare("bad integer"))
            }
        }
        Value::Bool(value) => Ok(i64::from(*value)),
        Value::String(text) => text
            .trim()
            .parse::<i64>()
            .map_err(|_| CompileError::bare(format!("bad integer {text}"))),
        _ => Err(CompileError::bare("bad integer")),
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                value != 0
            } else if let Some(value) = number.as_u64() {
                value != 0
            } else {
                number.as_f64().unwrap_or(0.0) != 0.0
            }
        }
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

fn py_str(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(text) => text.clone(),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                value.to_string()
            } else if let Some(value) = number.as_u64() {
                value.to_string()
            } else {
                number.to_string()
            }
        }
        other => other.to_string(),
    }
}
