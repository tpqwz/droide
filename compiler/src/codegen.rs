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
        self.dex.add(cls);
    }

    fn emit_natives(&mut self, checked: &Checked) {
        let mut cls = ClassModel::new(
            &self.native_desc,
            "Ljava/lang/Object;",
            ACC_PUBLIC,
            "Native",
        );
        let mut clinit = Asm::new();
        clinit.const_string(0, "droide");
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
            cls.field(&widget.name, spec.desc, ACC_PUBLIC);
        }
        cls.method(activity_init());
        cls.method(self.on_create(form, &desc)?);
        for rec in by_index(&form.routines) {
            cls.method(self.compile_routine(rec, &desc, false)?);
        }
        self.dex.add(cls);
        for widget in ordered_widgets(form) {
            if let Some(handler) = event_of(&widget.comp, "OnClick") {
                self.dex.add(self.listener(form, &desc, widget, handler)?);
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
        let outs = {
            let mut gen = Gen::new(
                &mut asm,
                &frame,
                &mut self.lab,
                owner,
                &package,
                &native_desc,
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
            width = width.max(comp_int(comp, "left")?.saturating_add(comp_int(comp, "width")?));
            height = height.max(comp_int(comp, "top")?.saturating_add(comp_int(comp, "height")?));
        }
        self.place_frame(&mut asm, this, width, height);
        for comp in comps {
            self.place_widget(&mut asm, form, desc, this, comp)?;
        }
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
        // The horizontal ProgressBar constructor takes four words, so outs is 4.
        Ok(MethodModel::new(
            "onCreate",
            "V",
            vec!["Landroid/os/Bundle;".into()],
            ACC_PUBLIC,
            Some(asm),
            8,
            2,
            4,
        ))
    }

    fn place_frame(&self, asm: &mut Asm, this: u16, width: i64, height: i64) {
        self.emit_dp(asm, this, 4, width);
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
                if let Some(value) = map.get(*prop) {
                    emit_const_prop(asm, 2, prop, value)?;
                }
            }
        }
        if kind == "Date" {
            emit_update_date(asm, props)?;
        }
        self.emit_dp(asm, this, 4, comp_int(comp, "width")?);
        self.emit_dp(asm, this, 5, comp_int(comp, "height")?);
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
        self.emit_dp(asm, this, 4, comp_int(comp, "left")?);
        self.emit_dp(asm, this, 5, comp_int(comp, "top")?);
        asm.iput(
            4,
            3,
            field(
                "Landroid/view/ViewGroup$MarginLayoutParams;",
                "leftMargin",
                "I",
            ),
        );
        asm.iput(
            5,
            3,
            field(
                "Landroid/view/ViewGroup$MarginLayoutParams;",
                "topMargin",
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
        asm.iput_obj(2, this, field(desc, comp_name, wdesc));
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
}

impl<'a> Gen<'a> {
    fn new(
        asm: &'a mut Asm,
        frame: &'a Frame,
        lab: &'a mut u32,
        owner: &str,
        package: &str,
        native_desc: &str,
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
            sp: 0,
            outs: 0,
            ret_label,
        }
    }

    fn fresh(&mut self) -> String {
        *self.lab += 1;
        format!("L{}", self.lab)
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
                self.emit_setter(wreg, &actual, vreg)?;
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
        self.asm
            .iget_obj(dest, this, field(&self.owner, &widget.name, spec.desc));
        Ok(())
    }

    fn emit_setter(&mut self, wreg: u16, prop: &str, vreg: u16) -> Result<(), CompileError> {
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
        let mut arg_regs = Vec::new();
        for arg in args {
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
            "OpenForm" => {
                let form_name = open_form_name(form_name, args)?;
                let this = self
                    .frame
                    .this
                    .ok_or_else(|| CompileError::bare("OpenForm outside a form"))?;
                let intent = self.alloc()?;
                self.asm.new_instance(intent, "Landroid/content/Intent;");
                self.invoke(
                    "direct",
                    method("Landroid/content/Intent;", "<init>", "V", &[]),
                    &[intent],
                );
                let cls = self.alloc()?;
                self.asm
                    .const_class(cls, &class_desc(&self.package, &form_name));
                self.invoke(
                    "virtual",
                    method(
                        "Landroid/content/Intent;",
                        "setClass",
                        "Landroid/content/Intent;",
                        &["Landroid/content/Context;", "Ljava/lang/Class;"],
                    ),
                    &[intent, this, cls],
                );
                self.asm.move_result(intent, true);
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
