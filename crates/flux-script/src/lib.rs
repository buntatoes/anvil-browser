//! Page scripting runtime and DOM bindings.
//!
//! Executes inline `<script>` blocks and `click` event listeners against a
//! [`Document`].

use flux_dom::{Document, NodeId};
use flux_json::{JsonValue, parse_json, to_json_string};

#[cfg(flux_cpp_jit)]
unsafe extern "C" {
    fn flux_cpp_jit_kernel_version() -> u32;
    fn flux_cpp_jit_eval_binary(opcode: u32, lhs: f64, rhs: f64) -> f64;
}

/// True when `flux_jit_accel.cpp` was linked.
pub fn is_cpp_jit_kernel_active() -> bool {
    #[cfg(flux_cpp_jit)]
    {
        unsafe { flux_cpp_jit_kernel_version() == 16 }
    }
    #[cfg(not(flux_cpp_jit))]
    {
        false
    }
}

fn eval_jit_binary_op(opcode: u32, lhs: f64, rhs: f64) -> f64 {
    #[cfg(flux_cpp_jit)]
    unsafe {
        return flux_cpp_jit_eval_binary(opcode, lhs, rhs);
    }
    #[cfg(not(flux_cpp_jit))]
    match opcode {
        0 => lhs + rhs,
        1 => lhs - rhs,
        2 => lhs * rhs,
        3 => {
            if rhs == 0.0 {
                0.0
            } else {
                lhs / rhs
            }
        }
        4 => {
            let r = rhs as i64;
            if r == 0 {
                0.0
            } else {
                ((lhs as i64) % r) as f64
            }
        }
        _ => 0.0,
    }
}

/// Cached script statement.
#[derive(Clone, Debug, PartialEq)]
pub enum CompiledOp {
    SetVarConst { name: String, value: Value },
    SetVarExpr { name: String, expr: String },
    AssignTarget { lhs: String, expr: String },
    SetTitleConst(String),
    EvalStatement(String),
}

/// Counts from the statement cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JitStats {
    pub jit_enabled: bool,
    pub cpp_kernel_linked: bool,
    pub compiled_blocks: usize,
    pub compiled_ops_executed: usize,
    pub folded_constants: usize,
    pub inline_cache_hits: usize,
}

impl Default for JitStats {
    fn default() -> Self {
        Self {
            jit_enabled: true,
            cpp_kernel_linked: is_cpp_jit_kernel_active(),
            compiled_blocks: 0,
            compiled_ops_executed: 0,
            folded_constants: 0,
            inline_cache_hits: 0,
        }
    }
}

/// Maximum statement/expression evaluation steps per script invocation to prevent UI hangs.
pub const MAX_EXECUTION_STEPS: usize = 100_000;

/// Maximum function call recursion depth allowed in page scripts.
pub const MAX_CALL_DEPTH: usize = 64;

/// Maximum number of elements allowed in a script array literal.
pub const MAX_ARRAY_LEN: usize = 10_000;

/// Maximum queued timers allowed per script turn.
pub const MAX_TIMERS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Element(NodeId),
    Array(Vec<Value>),
    Json(JsonValue),
}

impl Value {
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(b) => *b,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::Str(s) => !s.is_empty(),
            Self::Element(_) | Self::Array(_) | Self::Json(_) => true,
        }
    }

    pub fn as_bool(&self) -> bool {
        self.is_truthy()
    }

    pub fn as_string(&self, doc: &Document) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Bool(b) => b.to_string(),
            Self::Number(n) => {
                if n.fract() == 0.0 {
                    format!("{}", *n as i64)
                } else {
                    n.to_string()
                }
            }
            Self::Str(s) => s.clone(),
            Self::Element(id) => doc
                .get(*id)
                .data()
                .tag()
                .map(|t| format!("<{t}>"))
                .unwrap_or_else(|| "<node>".to_string()),
            Self::Array(items) => items
                .iter()
                .map(|v| v.as_string(doc))
                .collect::<Vec<_>>()
                .join(","),
            Self::Json(j) => to_json_string(j),
        }
    }

    fn to_json_value(&self, doc: &Document) -> JsonValue {
        match self {
            Self::Null => JsonValue::Null,
            Self::Bool(b) => JsonValue::Bool(*b),
            Self::Number(n) => JsonValue::Number(*n),
            Self::Str(s) => JsonValue::String(s.clone()),
            Self::Element(_) => JsonValue::String(self.as_string(doc)),
            Self::Array(items) => {
                JsonValue::Array(items.iter().map(|v| v.to_json_value(doc)).collect())
            }
            Self::Json(j) => j.clone(),
        }
    }

    fn from_json_value(j: JsonValue) -> Self {
        match j {
            JsonValue::Null => Self::Null,
            JsonValue::Bool(b) => Self::Bool(b),
            JsonValue::Number(n) => Self::Number(n),
            JsonValue::String(s) => Self::Str(s),
            JsonValue::Array(items) => {
                Self::Array(items.into_iter().map(Self::from_json_value).collect())
            }
            other => Self::Json(other),
        }
    }

    fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            Self::Str(s) => s.trim().parse().ok(),
            Self::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptRuntime {
    vars: Vec<(String, Value)>,
    functions: Vec<(String, Vec<String>, String)>,
    click_listeners: Vec<(NodeId, String)>,
    timers: Vec<(u64, String)>,
    fetch_mocks: Vec<(String, u16, String)>,
    console_logs: Vec<String>,
    dom_cache: Vec<(String, NodeId)>,
    jit_stats: JitStats,
    return_value: Option<Value>,
    pending_navigation: Option<String>,
    page_origin: Option<String>,
    steps: usize,
    call_depth: usize,
    aborted: bool,
}

impl Default for ScriptRuntime {
    fn default() -> Self {
        Self {
            vars: Vec::new(),
            functions: Vec::new(),
            click_listeners: Vec::new(),
            timers: Vec::new(),
            fetch_mocks: Vec::new(),
            console_logs: Vec::new(),
            dom_cache: Vec::new(),
            jit_stats: JitStats::default(),
            return_value: None,
            pending_navigation: None,
            page_origin: None,
            steps: 0,
            call_depth: 0,
            aborted: false,
        }
    }
}

impl ScriptRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_jit_enabled(jit_enabled: bool) -> Self {
        Self::new_with_jit(jit_enabled)
    }

    pub fn new_with_jit(jit_enabled: bool) -> Self {
        let mut rt = Self::default();
        rt.set_jit_enabled(jit_enabled);
        rt
    }

    pub fn set_jit_enabled(&mut self, enabled: bool) {
        self.jit_stats.jit_enabled = enabled;
        if !enabled {
            self.dom_cache.clear();
        }
    }

    pub fn is_jit_enabled(&self) -> bool {
        self.jit_stats.jit_enabled
    }

    pub fn jit_stats(&self) -> &JitStats {
        &self.jit_stats
    }

    pub fn console_logs(&self) -> &[String] {
        &self.console_logs
    }

    pub fn take_pending_navigation(&mut self) -> Option<String> {
        self.pending_navigation.take()
    }

    pub fn set_page_origin(&mut self, origin: &str) {
        let trimmed = origin.trim();
        if trimmed.is_empty() {
            self.page_origin = None;
        } else {
            self.page_origin = Some(trimmed.to_string());
        }
    }

    pub fn was_aborted(&self) -> bool {
        self.aborted
    }

    pub fn reset_execution_budget(&mut self) {
        self.steps = 0;
        self.call_depth = 0;
        self.aborted = false;
    }

    fn consume_step(&mut self) -> bool {
        if self.aborted {
            return false;
        }
        self.steps = self.steps.saturating_add(1);
        if self.steps > MAX_EXECUTION_STEPS {
            self.aborted = true;
            return false;
        }
        true
    }

    pub fn register_fetch_response(&mut self, url: &str, status: u16, body: &str) {
        self.fetch_mocks
            .push((url.to_string(), status, body.to_string()));
    }

    pub fn run_timers(&mut self, doc: &mut Document) -> usize {
        let mut pending = std::mem::take(&mut self.timers);
        pending.sort_by_key(|(delay, _)| *delay);
        let count = pending.len();
        for (_, code) in pending {
            if self.aborted {
                break;
            }
            self.eval_script(doc, &code);
        }
        count
    }

    pub fn get_var(&self, name: &str) -> Option<&Value> {
        self.vars
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    }

    pub fn set_var(&mut self, name: &str, value: Value) {
        if let Some((_, slot)) = self.vars.iter_mut().rev().find(|(k, _)| k == name) {
            *slot = value;
        } else {
            self.vars.push((name.to_string(), value));
        }
    }

    pub fn run_document_scripts(&mut self, doc: &mut Document) {
        self.reset_execution_budget();
        self.collect_onclick_attrs(doc, doc.root());
        for script in doc.scripts() {
            if self.aborted {
                break;
            }
            self.eval_script(doc, &script);
        }
        let _ = self.run_timers(doc);
    }

    pub fn dispatch_click(&mut self, doc: &mut Document, target: NodeId) -> bool {
        self.reset_execution_budget();
        let mut cursor = Some(target);
        let mut handled = false;
        while let Some(id) = cursor {
            let handlers: Vec<String> = self
                .click_listeners
                .iter()
                .filter(|(node, _)| *node == id)
                .map(|(_, body)| body.clone())
                .collect();
            for body in handlers {
                self.eval_script(doc, &body);
                handled = true;
            }
            cursor = doc.get(id).parent();
        }
        if handled {
            let _ = self.run_timers(doc);
        }
        handled
    }

    pub fn eval_script(&mut self, doc: &mut Document, source: &str) {
        if self.aborted {
            return;
        }
        let cleaned = strip_comments(source);
        let statements = split_statements(&cleaned);
        for stmt in &statements {
            if stmt.trim_start().starts_with("function ") {
                self.exec_statement(doc, stmt);
            }
        }
        if self.jit_stats.jit_enabled {
            let compiled = self.compile_jit_block(&statements);
            self.jit_stats.compiled_blocks = self.jit_stats.compiled_blocks.saturating_add(1);
            for op in compiled {
                if self.aborted || self.return_value.is_some() {
                    break;
                }
                self.exec_compiled_op(doc, op);
            }
        } else {
            for stmt in statements {
                if self.aborted || self.return_value.is_some() {
                    break;
                }
                if !stmt.trim_start().starts_with("function ") {
                    self.exec_statement(doc, &stmt);
                }
            }
        }
    }

    fn compile_jit_block(&mut self, statements: &[String]) -> Vec<CompiledOp> {
        let mut ops = Vec::with_capacity(statements.len());
        for raw in statements {
            let stmt = raw.trim();
            if stmt.is_empty() || stmt.starts_with("function ") {
                continue;
            }
            if stmt.starts_with("if")
                || stmt.starts_with("for")
                || stmt.starts_with("while")
                || stmt.starts_with("try")
                || stmt.starts_with("return")
                || stmt.contains("=>")
                || stmt.contains("+=")
                || stmt.contains("-=")
                || stmt.ends_with("++")
                || stmt.ends_with("--")
            {
                ops.push(CompiledOp::EvalStatement(stmt.to_string()));
                continue;
            }
            let is_decl =
                stmt.starts_with("let ") || stmt.starts_with("const ") || stmt.starts_with("var ");
            let stripped = stmt
                .strip_prefix("let ")
                .or_else(|| stmt.strip_prefix("const "))
                .or_else(|| stmt.strip_prefix("var "))
                .unwrap_or(stmt)
                .trim();

            if let Some((lhs, rhs)) = split_assignment(stripped) {
                let lhs = lhs.trim();
                let rhs = rhs.trim();
                if lhs == "document.title" {
                    if let Some(Value::Str(s)) = self.try_fold_constant_expr(rhs) {
                        ops.push(CompiledOp::SetTitleConst(s));
                        continue;
                    }
                }
                if is_decl && is_identifier(lhs) {
                    if let Some(const_val) = self.try_fold_constant_expr(rhs) {
                        ops.push(CompiledOp::SetVarConst {
                            name: lhs.to_string(),
                            value: const_val,
                        });
                        continue;
                    }
                    ops.push(CompiledOp::SetVarExpr {
                        name: lhs.to_string(),
                        expr: rhs.to_string(),
                    });
                    continue;
                }
                ops.push(CompiledOp::AssignTarget {
                    lhs: lhs.to_string(),
                    expr: rhs.to_string(),
                });
                continue;
            }
            ops.push(CompiledOp::EvalStatement(stmt.to_string()));
        }
        ops
    }

    fn try_fold_constant_expr(&mut self, expr: &str) -> Option<Value> {
        let e = expr.trim();
        if let Some(s) = parse_string_literal(e) {
            return Some(Value::Str(s));
        }
        if e == "true" {
            return Some(Value::Bool(true));
        }
        if e == "false" {
            return Some(Value::Bool(false));
        }
        if let Ok(n) = e.parse::<f64>() {
            return Some(Value::Number(n));
        }
        for (op_str, opcode) in [
            ("+", 0u32),
            ("-", 1u32),
            ("*", 2u32),
            ("/", 3u32),
            ("%", 4u32),
        ] {
            if let Some((l_str, r_str)) = split_top_level_op(e, op_str) {
                if !l_str.trim().is_empty() {
                    if let (Ok(l_num), Ok(r_num)) =
                        (l_str.trim().parse::<f64>(), r_str.trim().parse::<f64>())
                    {
                        let folded = eval_jit_binary_op(opcode, l_num, r_num);
                        self.jit_stats.folded_constants =
                            self.jit_stats.folded_constants.saturating_add(1);
                        return Some(Value::Number(folded));
                    }
                }
            }
        }
        None
    }

    fn exec_compiled_op(&mut self, doc: &mut Document, op: CompiledOp) {
        if !self.consume_step() {
            return;
        }
        self.jit_stats.compiled_ops_executed =
            self.jit_stats.compiled_ops_executed.saturating_add(1);
        match op {
            CompiledOp::SetVarConst { name, value } => {
                self.set_var(&name, value);
            }
            CompiledOp::SetVarExpr { name, expr } => {
                let val = self.eval_expr(doc, &expr);
                self.set_var(&name, val);
            }
            CompiledOp::AssignTarget { lhs, expr } => {
                let val = self.eval_expr(doc, &expr);
                self.assign_target(doc, &lhs, val);
            }
            CompiledOp::SetTitleConst(title) => {
                doc.set_title(&title);
            }
            CompiledOp::EvalStatement(stmt) => {
                self.steps = self.steps.saturating_sub(1);
                self.exec_statement(doc, &stmt);
            }
        }
    }

    fn collect_onclick_attrs(&mut self, doc: &Document, id: NodeId) {
        if let Some(code) = doc.get(id).data().attr("onclick") {
            let code = code.trim();
            if !code.is_empty() {
                self.click_listeners.push((id, code.to_string()));
            }
        }
        let children: Vec<NodeId> = doc.children(id).to_vec();
        for child in children {
            self.collect_onclick_attrs(doc, child);
        }
    }

    fn exec_statement(&mut self, doc: &mut Document, raw: &str) {
        if !self.consume_step() {
            return;
        }
        let stmt = raw.trim();
        if stmt.is_empty() {
            return;
        }

        if stmt == "return" {
            self.return_value = Some(Value::Null);
            return;
        }
        if let Some(ret_expr) = stmt.strip_prefix("return ") {
            let val = self.eval_expr(doc, ret_expr.trim());
            self.return_value = Some(val);
            return;
        }

        if let Some(rest) = stmt.strip_prefix("try") {
            if let Some((try_body, after_try)) = extract_first_brace_block(rest) {
                self.eval_script(doc, &try_body);
                if self.aborted {
                    self.aborted = false;
                    let after_try = after_try.trim();
                    if let Some(catch_rest) = after_try.strip_prefix("catch") {
                        let catch_rest = catch_rest.trim_start();
                        let catch_body_src = if catch_rest.starts_with('(') {
                            extract_paren_group(catch_rest)
                                .map(|(_, rem)| rem)
                                .unwrap_or(catch_rest)
                        } else {
                            catch_rest
                        };
                        if let Some((catch_body, _)) = extract_first_brace_block(catch_body_src) {
                            self.eval_script(doc, &catch_body);
                        }
                    }
                }
                return;
            }
        }

        if let Some(rest) = stmt.strip_prefix("function ") {
            if let Some((sig, body)) = split_brace_block(rest) {
                if let Some((name, params_raw)) = sig.split_once('(') {
                    let name = name.trim().to_string();
                    let params_str = params_raw.trim().trim_end_matches(')');
                    let params: Vec<String> = params_str
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect();
                    if !name.is_empty() {
                        self.functions.push((name, params, body));
                    }
                }
            }
            return;
        }

        if let Some(rest) = stmt.strip_prefix("while") {
            let rest = rest.trim_start();
            if rest.starts_with('(') {
                if let Some((cond_str, after_cond)) = extract_paren_group(rest) {
                    if let Some((loop_body, _)) = extract_first_brace_block(after_cond) {
                        for _ in 0..10_000 {
                            if self.aborted || !self.eval_expr(doc, &cond_str).is_truthy() {
                                break;
                            }
                            self.eval_script(doc, &loop_body);
                        }
                        return;
                    }
                }
            }
        }

        if let Some(rest) = stmt.strip_prefix("for") {
            let rest = rest.trim_start();
            if rest.starts_with('(') {
                if let Some((header_str, after_header)) = extract_paren_group(rest) {
                    if let Some((loop_body, _)) = extract_first_brace_block(after_header) {
                        let parts: Vec<&str> = header_str.splitn(3, ';').collect();
                        if parts.len() == 3 {
                            self.exec_statement(doc, parts[0]);
                            for _ in 0..10_000 {
                                if self.aborted {
                                    break;
                                }
                                if !parts[1].trim().is_empty()
                                    && !self.eval_expr(doc, parts[1]).is_truthy()
                                {
                                    break;
                                }
                                self.eval_script(doc, &loop_body);
                                self.exec_statement(doc, parts[2]);
                            }
                            return;
                        }
                    }
                }
            }
        }

        if let Some(rest) = stmt.strip_prefix("if") {
            let rest = rest.trim_start();
            if rest.starts_with('(') {
                if let Some((cond_str, after_cond)) = extract_paren_group(rest) {
                    if let Some((then_body, after_then)) = extract_first_brace_block(after_cond) {
                        let cond_val = self.eval_expr(doc, &cond_str);
                        if cond_val.is_truthy() {
                            self.eval_script(doc, &then_body);
                        } else {
                            let after_then = after_then.trim();
                            if let Some(else_rest) = after_then.strip_prefix("else") {
                                if let Some((else_body, _)) = extract_first_brace_block(else_rest) {
                                    self.eval_script(doc, &else_body);
                                } else {
                                    self.exec_statement(doc, else_rest);
                                }
                            }
                        }
                        return;
                    }
                }
            }
        }

        let stripped = stmt
            .strip_prefix("let ")
            .or_else(|| stmt.strip_prefix("const "))
            .or_else(|| stmt.strip_prefix("var "))
            .unwrap_or(stmt)
            .trim();

        if let Some(ident) = stripped.strip_suffix("++") {
            let ident = ident.trim();
            if is_identifier(ident) {
                let cur = self
                    .get_var(ident)
                    .and_then(Value::as_number)
                    .unwrap_or(0.0);
                self.set_var(ident, Value::Number(cur + 1.0));
                return;
            }
        }
        if let Some(ident) = stripped.strip_suffix("--") {
            let ident = ident.trim();
            if is_identifier(ident) {
                let cur = self
                    .get_var(ident)
                    .and_then(Value::as_number)
                    .unwrap_or(0.0);
                self.set_var(ident, Value::Number(cur - 1.0));
                return;
            }
        }
        if let Some((lhs, rhs)) = stripped.split_once("+=") {
            let lhs = lhs.trim();
            let cur = self.eval_expr(doc, lhs);
            let add = self.eval_expr(doc, rhs.trim());
            let next = match (&cur, &add) {
                (Value::Number(a), Value::Number(b)) => Value::Number(a + b),
                _ => Value::Str(format!("{}{}", cur.as_string(doc), add.as_string(doc))),
            };
            self.assign_target(doc, lhs, next);
            return;
        }
        if let Some((lhs, rhs)) = stripped.split_once("-=") {
            let lhs = lhs.trim();
            let cur = self.eval_expr(doc, lhs).as_number().unwrap_or(0.0);
            let sub = self.eval_expr(doc, rhs.trim()).as_number().unwrap_or(0.0);
            self.assign_target(doc, lhs, Value::Number(cur - sub));
            return;
        }

        if let Some((lhs, rhs)) = split_assignment(stripped) {
            let lhs_trimmed = lhs.trim();
            let rhs_trimmed = rhs.trim();
            if is_identifier(lhs_trimmed) {
                if let Some((params_part, arrow_body)) = rhs_trimmed.split_once("=>") {
                    let params_str = params_part
                        .trim()
                        .trim_start_matches('(')
                        .trim_end_matches(')');
                    let params: Vec<String> = params_str
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect();
                    let arrow_body = arrow_body.trim();
                    let body = if let Some((_, block)) = split_brace_block(arrow_body) {
                        block
                    } else {
                        format!("return {arrow_body}")
                    };
                    self.functions.push((lhs_trimmed.to_string(), params, body));
                    return;
                }
            }
            let value = self.eval_expr(doc, rhs_trimmed);
            self.assign_target(doc, lhs_trimmed, value);
            return;
        }

        let _ = self.eval_expr(doc, stripped);
    }

    fn assign_target(&mut self, doc: &mut Document, lhs: &str, value: Value) {
        if lhs == "document.title" {
            let text = value.as_string(doc);
            doc.set_title(&text);
            return;
        }
        if lhs == "window.location.href" || lhs == "location.href" || lhs == "window.location" {
            let dest = value.as_string(doc);
            if !dest.trim().is_empty() {
                self.pending_navigation = Some(dest.trim().to_string());
            }
            return;
        }

        if let Some((obj_expr, prop)) = lhs.rsplit_once('.') {
            if let Some((el_expr, style_lit)) = obj_expr.rsplit_once('.') {
                if style_lit.trim() == "style" {
                    if let Value::Element(id) = self.eval_expr(doc, el_expr.trim()) {
                        let css_prop = camel_to_kebab(prop.trim());
                        let css_val = value.as_string(doc);
                        update_inline_style(doc, id, &css_prop, &css_val);
                    }
                    return;
                }
            }
            if let Value::Element(id) = self.eval_expr(doc, obj_expr.trim()) {
                match prop.trim() {
                    "textContent" | "innerText" => {
                        let text = value.as_string(doc);
                        doc.set_text_content(id, &text);
                    }
                    "innerHTML" => {
                        let html = value.as_string(doc);
                        doc.set_inner_html(id, &html);
                        self.dom_cache.clear();
                    }
                    "value" => {
                        let text = value.as_string(doc);
                        doc.set_input_value(id, &text);
                    }
                    "checked" => {
                        doc.set_checked(id, value.is_truthy());
                    }
                    "id" => {
                        let text = value.as_string(doc);
                        doc.set_attribute(id, "id", &text);
                        self.dom_cache.clear();
                    }
                    "className" => {
                        let text = value.as_string(doc);
                        doc.set_attribute(id, "class", &text);
                        self.dom_cache.clear();
                    }
                    _ => {}
                }
                return;
            }
        }

        if is_identifier(lhs) {
            self.set_var(lhs, value);
        }
    }

    pub fn eval_expr(&mut self, doc: &mut Document, raw: &str) -> Value {
        if !self.consume_step() {
            return Value::Null;
        }
        let expr = raw.trim();
        if expr.is_empty() {
            return Value::Null;
        }

        if expr.starts_with('[') && expr.ends_with(']') {
            let inner = &expr[1..expr.len() - 1];
            let items: Vec<Value> = split_args(inner)
                .into_iter()
                .take(MAX_ARRAY_LEN)
                .map(|item| self.eval_expr(doc, &item))
                .collect();
            return Value::Array(items);
        }

        if expr.starts_with('{') && expr.ends_with('}') {
            let inner = &expr[1..expr.len() - 1];
            let mut obj = JsonValue::empty_object();
            for pair in split_args(inner) {
                if let Some((k_raw, v_raw)) = pair.split_once(':') {
                    let key = parse_string_literal(k_raw.trim())
                        .unwrap_or_else(|| k_raw.trim().to_string());
                    let val = self.eval_expr(doc, v_raw.trim()).to_json_value(doc);
                    obj.insert(&key, val);
                }
            }
            return Value::Json(obj);
        }

        if expr.starts_with('`') && expr.ends_with('`') && expr.len() >= 2 {
            let inner = &expr[1..expr.len() - 1];
            return Value::Str(self.eval_template_literal(doc, inner));
        }

        if let Some(rest) = expr.strip_prefix("typeof ") {
            let v = self.eval_expr(doc, rest.trim());
            let ty = match v {
                Value::Null => "undefined",
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::Str(_) => "string",
                Value::Element(_) | Value::Array(_) | Value::Json(_) => "object",
            };
            return Value::Str(ty.to_string());
        }

        if let Some(rest) = expr.strip_prefix('!') {
            if !rest.starts_with('=') {
                let v = self.eval_expr(doc, rest.trim());
                return Value::Bool(!v.is_truthy());
            }
        }

        if let Some((cond_s, branches_s)) = split_top_level_op(expr, "?") {
            if let Some((then_s, else_s)) = split_top_level_op(branches_s, ":") {
                if self.eval_expr(doc, cond_s).is_truthy() {
                    return self.eval_expr(doc, then_s);
                } else {
                    return self.eval_expr(doc, else_s);
                }
            }
        }

        if let Some((left, right)) = split_top_level_op(expr, "||") {
            let l = self.eval_expr(doc, left);
            if l.is_truthy() {
                return l;
            }
            return self.eval_expr(doc, right);
        }

        if let Some((left, right)) = split_top_level_op(expr, "&&") {
            let l = self.eval_expr(doc, left);
            if !l.is_truthy() {
                return Value::Bool(false);
            }
            return self.eval_expr(doc, right);
        }

        for op in ["===", "!==", "==", "!=", "<=", ">=", "<", ">"] {
            if let Some((left, right)) = split_top_level_op(expr, op) {
                let l = self.eval_expr(doc, left);
                let r = self.eval_expr(doc, right);
                let res = match op {
                    "===" | "==" => values_equal(&l, &r, doc),
                    "!==" | "!=" => !values_equal(&l, &r, doc),
                    "<" => l.as_number().zip(r.as_number()).is_some_and(|(a, b)| a < b),
                    "<=" => l
                        .as_number()
                        .zip(r.as_number())
                        .is_some_and(|(a, b)| a <= b),
                    ">" => l.as_number().zip(r.as_number()).is_some_and(|(a, b)| a > b),
                    ">=" => l
                        .as_number()
                        .zip(r.as_number())
                        .is_some_and(|(a, b)| a >= b),
                    _ => false,
                };
                return Value::Bool(res);
            }
        }

        if let Some((left, right)) = split_top_level_op(expr, "+") {
            let l = self.eval_expr(doc, left);
            let r = self.eval_expr(doc, right);
            if let (Value::Number(a), Value::Number(b)) = (&l, &r) {
                return Value::Number(a + b);
            }
            return Value::Str(format!("{}{}", l.as_string(doc), r.as_string(doc)));
        }

        for op in ["-", "*", "/"] {
            if let Some((left, right)) = split_top_level_op(expr, op) {
                if left.trim().is_empty() && op == "-" {
                    let r = self.eval_expr(doc, right);
                    return Value::Number(-r.as_number().unwrap_or(0.0));
                }
                let l = self.eval_expr(doc, left).as_number().unwrap_or(0.0);
                let r = self.eval_expr(doc, right).as_number().unwrap_or(0.0);
                let out = match op {
                    "-" => l - r,
                    "*" => l * r,
                    "/" => {
                        if r == 0.0 {
                            0.0
                        } else {
                            l / r
                        }
                    }
                    _ => 0.0,
                };
                return Value::Number(out);
            }
        }

        if let Some(str_val) = parse_string_literal(expr) {
            return Value::Str(str_val);
        }
        if expr == "true" {
            return Value::Bool(true);
        }
        if expr == "false" {
            return Value::Bool(false);
        }
        if expr == "null" || expr == "undefined" {
            return Value::Null;
        }
        if let Ok(num) = expr.parse::<f64>() {
            return Value::Number(num);
        }
        if expr == "document.title" {
            return Value::Str(doc.title());
        }
        if expr == "document.body" {
            return Value::Element(doc.body());
        }
        if expr == "document.head" {
            return Value::Element(doc.head());
        }
        if expr == "window.location.href" || expr == "location.href" {
            return Value::Str(self.page_origin.clone().unwrap_or_default());
        }

        if expr.ends_with(')') {
            if let Some(open) = find_call_open_paren(expr) {
                let callee = expr[..open].trim();
                let args_src = &expr[open + 1..expr.len() - 1];
                return self.eval_call(doc, callee, args_src);
            }
        }

        if expr.ends_with(']') {
            if let Some(open) = expr.rfind('[') {
                if open > 0 {
                    let target_expr = expr[..open].trim();
                    let idx_expr = &expr[open + 1..expr.len() - 1];
                    let target = self.eval_expr(doc, target_expr);
                    let idx_val = self.eval_expr(doc, idx_expr);
                    match target {
                        Value::Array(items) => {
                            if let Some(i) = idx_val.as_number() {
                                return items
                                    .get(i.max(0.0) as usize)
                                    .cloned()
                                    .unwrap_or(Value::Null);
                            }
                        }
                        Value::Json(JsonValue::Array(items)) => {
                            if let Some(i) = idx_val.as_number() {
                                return items
                                    .get(i.max(0.0) as usize)
                                    .cloned()
                                    .map(Value::from_json_value)
                                    .unwrap_or(Value::Null);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if let Some((obj_expr, prop)) = expr.rsplit_once('.') {
            match self.eval_expr(doc, obj_expr.trim()) {
                Value::Element(id) => {
                    return match prop.trim() {
                        "textContent" | "innerText" => Value::Str(doc.text_content(id)),
                        "value" => Value::Str(doc.input_value(id)),
                        "checked" => Value::Bool(doc.is_checked(id)),
                        "id" => Value::Str(doc.get(id).data().id().unwrap_or("").to_string()),
                        "className" => {
                            Value::Str(doc.get(id).data().attr("class").unwrap_or("").to_string())
                        }
                        _ => Value::Null,
                    };
                }
                Value::Array(items) => {
                    if prop.trim() == "length" {
                        return Value::Number(items.len() as f64);
                    }
                    return Value::Null;
                }
                Value::Json(jval) => {
                    if prop.trim() == "length" {
                        if let JsonValue::Array(items) = &jval {
                            return Value::Number(items.len() as f64);
                        }
                    }
                    return jval
                        .get(prop.trim())
                        .cloned()
                        .map(Value::from_json_value)
                        .unwrap_or(Value::Null);
                }
                Value::Str(s) if prop.trim() == "length" => {
                    return Value::Number(s.chars().count() as f64);
                }
                _ => {}
            }
        }

        if let Some(val) = self.get_var(expr) {
            return val.clone();
        }

        Value::Null
    }

    fn eval_template_literal(&mut self, doc: &mut Document, inner: &str) -> String {
        let mut out = String::new();
        let mut rest = inner;
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            if let Some(end) = after.find('}') {
                let expr_str = &after[..end];
                let val = self.eval_expr(doc, expr_str);
                out.push_str(&val.as_string(doc));
                rest = &after[end + 1..];
            } else {
                out.push_str(&rest[start..]);
                return out;
            }
        }
        out.push_str(rest);
        out
    }

    fn lookup_cached_dom(&mut self, key: &str) -> Option<NodeId> {
        if !self.jit_stats.jit_enabled {
            return None;
        }
        if let Some((_, id)) = self.dom_cache.iter().find(|(k, _)| k == key) {
            self.jit_stats.inline_cache_hits = self.jit_stats.inline_cache_hits.saturating_add(1);
            return Some(*id);
        }
        None
    }

    fn store_cached_dom(&mut self, key: String, id: NodeId) {
        if self.jit_stats.jit_enabled && self.dom_cache.len() < 128 {
            self.dom_cache.push((key, id));
        }
    }

    fn eval_call(&mut self, doc: &mut Document, callee: &str, args_src: &str) -> Value {
        match callee {
            "parseInt" | "parseFloat" | "Number" => {
                let args = split_args(args_src);
                let raw = args
                    .first()
                    .map(|a| self.eval_expr(doc, a).as_string(doc))
                    .unwrap_or_default();
                let trimmed = raw.trim();
                let num = if callee == "parseInt" {
                    trimmed
                        .split(|c: char| !c.is_ascii_digit() && c != '-' && c != '+')
                        .next()
                        .and_then(|s| s.parse::<i64>().ok())
                        .map(|n| n as f64)
                        .unwrap_or(0.0)
                } else {
                    trimmed.parse::<f64>().unwrap_or(0.0)
                };
                return Value::Number(num);
            }
            "String" => {
                let args = split_args(args_src);
                let s = args
                    .first()
                    .map(|a| self.eval_expr(doc, a).as_string(doc))
                    .unwrap_or_default();
                return Value::Str(s);
            }
            "Boolean" => {
                let args = split_args(args_src);
                let b = args
                    .first()
                    .map(|a| self.eval_expr(doc, a).as_bool())
                    .unwrap_or(false);
                return Value::Bool(b);
            }
            "isNaN" => {
                let args = split_args(args_src);
                let is_nan = args
                    .first()
                    .map(|a| {
                        let v = self.eval_expr(doc, a);
                        match v {
                            Value::Number(n) => n.is_nan(),
                            Value::Str(s) => s.trim().parse::<f64>().is_err(),
                            _ => false,
                        }
                    })
                    .unwrap_or(true);
                return Value::Bool(is_nan);
            }
            "encodeURIComponent" => {
                let args = split_args(args_src);
                let raw = args
                    .first()
                    .map(|a| self.eval_expr(doc, a).as_string(doc))
                    .unwrap_or_default();
                let mut encoded = String::new();
                for b in raw.bytes() {
                    if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                        encoded.push(b as char);
                    } else {
                        encoded.push_str(&format!("%{:02X}", b));
                    }
                }
                return Value::Str(encoded);
            }
            "decodeURIComponent" => {
                let args = split_args(args_src);
                let raw = args
                    .first()
                    .map(|a| self.eval_expr(doc, a).as_string(doc))
                    .unwrap_or_default();
                return Value::Str(raw.replace("%20", " "));
            }
            _ => {}
        }

        if callee == "setTimeout" || callee == "requestAnimationFrame" {
            let args = split_args(args_src);
            if let Some(first) = args.first() {
                if self.timers.len() < MAX_TIMERS {
                    let handler_raw = first.trim();
                    let body = extract_callback_body(handler_raw);
                    let delay = args
                        .get(1)
                        .and_then(|d| self.eval_expr(doc, d).as_number())
                        .unwrap_or(0.0)
                        .max(0.0) as u64;
                    self.timers.push((delay, body));
                }
            }
            return Value::Number(self.timers.len() as f64);
        }

        if callee == "fetch" {
            let args = split_args(args_src);
            let url = args
                .first()
                .map(|a| self.eval_expr(doc, a).as_string(doc))
                .unwrap_or_default();
            if !is_allowed_script_fetch(self.page_origin.as_deref(), &url) {
                let mut obj = JsonValue::empty_object();
                obj.insert("url", JsonValue::String(url));
                obj.insert("status", JsonValue::Number(403.0));
                obj.insert("ok", JsonValue::Bool(false));
                obj.insert(
                    "body",
                    JsonValue::String(
                        "{\"error\":\"same-origin policy blocked fetch\"}".to_string(),
                    ),
                );
                return Value::Json(obj);
            }
            let (status, body) = self
                .fetch_mocks
                .iter()
                .rev()
                .find(|(u, _, _)| u == &url)
                .map(|(_, s, b)| (*s, b.clone()))
                .unwrap_or((200, "{}".to_string()));
            let mut obj = JsonValue::empty_object();
            obj.insert("url", JsonValue::String(url));
            obj.insert("status", JsonValue::Number(status as f64));
            obj.insert("ok", JsonValue::Bool((200..300).contains(&status)));
            obj.insert("body", JsonValue::String(body));
            return Value::Json(obj);
        }

        if let Some((obj_expr, method)) = callee.rsplit_once('.') {
            let obj_expr = obj_expr.trim();
            let method = method.trim();

            if obj_expr == "console" {
                let args = split_args(args_src);
                let line = args
                    .iter()
                    .map(|a| self.eval_expr(doc, a).as_string(doc))
                    .collect::<Vec<_>>()
                    .join(" ");
                if self.console_logs.len() < 256 {
                    let prefix = match method {
                        "warn" => "[warn] ",
                        "error" => "[error] ",
                        _ => "",
                    };
                    self.console_logs.push(format!("{prefix}{line}"));
                }
                return Value::Null;
            }

            if obj_expr == "Math" {
                let args = split_args(args_src);
                let first = args
                    .first()
                    .and_then(|a| self.eval_expr(doc, a).as_number())
                    .unwrap_or(0.0);
                let second = args
                    .get(1)
                    .and_then(|a| self.eval_expr(doc, a).as_number())
                    .unwrap_or(0.0);
                let res = match method {
                    "floor" => first.floor(),
                    "ceil" => first.ceil(),
                    "round" => first.round(),
                    "abs" => first.abs(),
                    "sqrt" => first.max(0.0).sqrt(),
                    "min" => {
                        if args.len() >= 2 {
                            first.min(second)
                        } else {
                            first
                        }
                    }
                    "max" => {
                        if args.len() >= 2 {
                            first.max(second)
                        } else {
                            first
                        }
                    }
                    "pow" => first.powf(second),
                    "random" => 0.42,
                    _ => 0.0,
                };
                return Value::Number(res);
            }

            if let Some((el_expr, sub_obj)) = obj_expr.rsplit_once('.') {
                if sub_obj.trim() == "classList" {
                    if let Value::Element(id) = self.eval_expr(doc, el_expr.trim()) {
                        let args = split_args(args_src);
                        let cls = args
                            .first()
                            .map(|a| self.eval_expr(doc, a).as_string(doc))
                            .unwrap_or_default();
                        return match method {
                            "add" => {
                                doc.add_class(id, &cls);
                                Value::Null
                            }
                            "remove" => {
                                doc.remove_class(id, &cls);
                                Value::Null
                            }
                            "toggle" => Value::Bool(doc.toggle_class(id, &cls)),
                            "contains" => Value::Bool(doc.get(id).data().has_class(&cls)),
                            _ => Value::Null,
                        };
                    }
                }
            }

            if obj_expr == "JSON" {
                let args = split_args(args_src);
                return match method {
                    "parse" => {
                        let first = args
                            .first()
                            .map(|a| self.eval_expr(doc, a).as_string(doc))
                            .unwrap_or_default();
                        parse_json(&first)
                            .map(Value::from_json_value)
                            .unwrap_or(Value::Null)
                    }
                    "stringify" => {
                        let first = args
                            .first()
                            .map(|a| self.eval_expr(doc, a).to_json_value(doc))
                            .unwrap_or(JsonValue::Null);
                        Value::Str(to_json_string(&first))
                    }
                    _ => Value::Null,
                };
            }

            if obj_expr == "navigator" {
                let args = split_args(args_src);
                let first = args
                    .first()
                    .map(|a| self.eval_expr(doc, a).as_string(doc))
                    .unwrap_or_default();
                return match method {
                    "requestMediaKeySystemAccess" => {
                        let lower = first.trim().to_ascii_lowercase();
                        let supported = matches!(
                            lower.as_str(),
                            "org.w3.clearkey"
                                | "org.flux.mediafoundation.cdm"
                                | "org.flux.external.cdm"
                        );
                        let mut obj = JsonValue::empty_object();
                        obj.insert("keySystem", JsonValue::String(lower));
                        obj.insert("supported", JsonValue::Bool(supported));
                        obj.insert("cenc", JsonValue::String("cenc".to_string()));
                        Value::Json(obj)
                    }
                    _ => Value::Null,
                };
            }

            if obj_expr == "document" {
                let args = split_args(args_src);
                let first = args
                    .first()
                    .map(|a| self.eval_expr(doc, a).as_string(doc))
                    .unwrap_or_default();
                return match method {
                    "getElementById" => {
                        let cache_key = format!("#{}", first);
                        if let Some(id) = self.lookup_cached_dom(&cache_key) {
                            return Value::Element(id);
                        }
                        if let Some(id) = doc.get_element_by_id(&first) {
                            self.store_cached_dom(cache_key, id);
                            Value::Element(id)
                        } else {
                            Value::Null
                        }
                    }
                    "querySelector" => {
                        if let Some(id) = self.lookup_cached_dom(&first) {
                            return Value::Element(id);
                        }
                        if let Some(id) = doc.query_selector(&first) {
                            self.store_cached_dom(first, id);
                            Value::Element(id)
                        } else {
                            Value::Null
                        }
                    }
                    "querySelectorAll" => {
                        let ids = doc.query_selector_all(&first);
                        Value::Array(ids.into_iter().map(Value::Element).collect())
                    }
                    "createElement" => {
                        let id = doc.create_element(&first, Vec::new());
                        Value::Element(id)
                    }
                    _ => Value::Null,
                };
            }

            if is_identifier(obj_expr) {
                if let Some(Value::Array(mut items)) = self.get_var(obj_expr).cloned() {
                    let args = split_args(args_src);
                    match method {
                        "push" => {
                            for arg in &args {
                                items.push(self.eval_expr(doc, arg));
                            }
                            let len = items.len();
                            self.set_var(obj_expr, Value::Array(items));
                            return Value::Number(len as f64);
                        }
                        "pop" => {
                            let popped = items.pop().unwrap_or(Value::Null);
                            self.set_var(obj_expr, Value::Array(items));
                            return popped;
                        }
                        _ => {}
                    }
                }
            }

            match self.eval_expr(doc, obj_expr) {
                Value::Str(s) => {
                    let args = split_args(args_src);
                    let arg0 = args
                        .first()
                        .map(|a| self.eval_expr(doc, a).as_string(doc))
                        .unwrap_or_default();
                    return match method {
                        "trim" => Value::Str(s.trim().to_string()),
                        "toLowerCase" => Value::Str(s.to_ascii_lowercase()),
                        "toUpperCase" => Value::Str(s.to_ascii_uppercase()),
                        "includes" => Value::Bool(s.contains(&arg0)),
                        "startsWith" => Value::Bool(s.starts_with(&arg0)),
                        "endsWith" => Value::Bool(s.ends_with(&arg0)),
                        "indexOf" => {
                            Value::Number(s.find(&arg0).map(|idx| idx as f64).unwrap_or(-1.0))
                        }
                        "charAt" => {
                            let idx = args
                                .first()
                                .and_then(|a| self.eval_expr(doc, a).as_number())
                                .unwrap_or(0.0)
                                .max(0.0) as usize;
                            Value::Str(
                                s.chars()
                                    .nth(idx)
                                    .map(|c| c.to_string())
                                    .unwrap_or_default(),
                            )
                        }
                        "replace" | "replaceAll" => {
                            let arg1 = args
                                .get(1)
                                .map(|a| self.eval_expr(doc, a).as_string(doc))
                                .unwrap_or_default();
                            if method == "replaceAll" {
                                Value::Str(s.replace(&arg0, &arg1))
                            } else {
                                Value::Str(s.replacen(&arg0, &arg1, 1))
                            }
                        }
                        "split" => {
                            let parts = if arg0.is_empty() {
                                s.chars().map(|c| Value::Str(c.to_string())).collect()
                            } else {
                                s.split(&arg0).map(|p| Value::Str(p.to_string())).collect()
                            };
                            Value::Array(parts)
                        }
                        "slice" | "substring" => {
                            let chars: Vec<char> = s.chars().collect();
                            let start = args
                                .first()
                                .and_then(|a| self.eval_expr(doc, a).as_number())
                                .unwrap_or(0.0)
                                .max(0.0) as usize;
                            let end = args
                                .get(1)
                                .and_then(|a| self.eval_expr(doc, a).as_number())
                                .map(|n| (n.max(0.0) as usize).min(chars.len()))
                                .unwrap_or(chars.len());
                            if start < end && start < chars.len() {
                                Value::Str(chars[start..end].iter().collect())
                            } else {
                                Value::Str(String::new())
                            }
                        }
                        _ => Value::Null,
                    };
                }
                Value::Array(items) => {
                    let args = split_args(args_src);
                    return match method {
                        "join" => {
                            let sep = args
                                .first()
                                .map(|a| self.eval_expr(doc, a).as_string(doc))
                                .unwrap_or_else(|| ",".to_string());
                            let joined = items
                                .iter()
                                .map(|v| v.as_string(doc))
                                .collect::<Vec<_>>()
                                .join(&sep);
                            Value::Str(joined)
                        }
                        "includes" => {
                            let needle = args
                                .first()
                                .map(|a| self.eval_expr(doc, a))
                                .unwrap_or(Value::Null);
                            Value::Bool(items.iter().any(|v| values_equal(v, &needle, doc)))
                        }
                        "indexOf" => {
                            let needle = args
                                .first()
                                .map(|a| self.eval_expr(doc, a))
                                .unwrap_or(Value::Null);
                            let idx = items
                                .iter()
                                .position(|v| values_equal(v, &needle, doc))
                                .map(|i| i as f64)
                                .unwrap_or(-1.0);
                            Value::Number(idx)
                        }
                        "slice" => {
                            let start = args
                                .first()
                                .and_then(|a| self.eval_expr(doc, a).as_number())
                                .unwrap_or(0.0)
                                .max(0.0) as usize;
                            let end = args
                                .get(1)
                                .and_then(|a| self.eval_expr(doc, a).as_number())
                                .map(|n| (n.max(0.0) as usize).min(items.len()))
                                .unwrap_or(items.len());
                            if start < end && start < items.len() {
                                Value::Array(items[start..end].to_vec())
                            } else {
                                Value::Array(Vec::new())
                            }
                        }
                        "forEach" => {
                            if let Some(cb) = args.first() {
                                let (param_name, body) = extract_single_param_callback(cb.trim());
                                for item in items {
                                    if !param_name.is_empty() {
                                        self.set_var(&param_name, item);
                                    }
                                    self.eval_script(doc, &body);
                                }
                            }
                            Value::Null
                        }
                        _ => Value::Null,
                    };
                }
                Value::Json(jval) => match method {
                    "text" => {
                        return jval
                            .get("body")
                            .and_then(|b| b.as_str())
                            .map(|s| Value::Str(s.to_string()))
                            .unwrap_or(Value::Str(String::new()));
                    }
                    "json" => {
                        let body = jval.get("body").and_then(|b| b.as_str()).unwrap_or("{}");
                        return parse_json(body)
                            .map(Value::from_json_value)
                            .unwrap_or(Value::Null);
                    }
                    _ => {}
                },
                Value::Element(id) => {
                    let args = split_args(args_src);
                    match method {
                        "getAttribute" => {
                            let name = args
                                .first()
                                .map(|a| self.eval_expr(doc, a).as_string(doc))
                                .unwrap_or_default();
                            return doc
                                .get(id)
                                .data()
                                .attr(&name)
                                .map(|v| Value::Str(v.to_string()))
                                .unwrap_or(Value::Null);
                        }
                        "setAttribute" if args.len() >= 2 => {
                            let name = self.eval_expr(doc, &args[0]).as_string(doc);
                            let value = self.eval_expr(doc, &args[1]).as_string(doc);
                            doc.set_attribute(id, &name, &value);
                            return Value::Null;
                        }
                        "removeAttribute" if !args.is_empty() => {
                            let name = self.eval_expr(doc, &args[0]).as_string(doc);
                            doc.set_attribute(id, &name, "");
                            return Value::Null;
                        }
                        "appendChild" if !args.is_empty() => {
                            if let Value::Element(child_id) = self.eval_expr(doc, &args[0]) {
                                let _ = doc.append(id, child_id);
                                self.dom_cache.clear();
                                return Value::Element(child_id);
                            }
                            return Value::Null;
                        }
                        "remove" => {
                            let _ = doc.remove_node(id);
                            self.dom_cache.clear();
                            return Value::Null;
                        }
                        "focus" | "blur" => {
                            return Value::Null;
                        }
                        "querySelector" => {
                            let sel = args
                                .first()
                                .map(|a| self.eval_expr(doc, a).as_string(doc))
                                .unwrap_or_default();
                            return doc
                                .query_selector(&sel)
                                .map(Value::Element)
                                .unwrap_or(Value::Null);
                        }
                        "querySelectorAll" => {
                            let sel = args
                                .first()
                                .map(|a| self.eval_expr(doc, a).as_string(doc))
                                .unwrap_or_default();
                            let ids = doc.query_selector_all(&sel);
                            return Value::Array(ids.into_iter().map(Value::Element).collect());
                        }
                        "addEventListener" if args.len() >= 2 => {
                            let event = self.eval_expr(doc, &args[0]).as_string(doc);
                            if event.eq_ignore_ascii_case("click") {
                                let handler_raw = args[1].trim();
                                let body = extract_callback_body(handler_raw);
                                self.click_listeners.push((id, body));
                            }
                            return Value::Null;
                        }
                        _ => return Value::Null,
                    }
                }
                _ => {}
            }
        }

        if let Some((_, params, body)) = self
            .functions
            .iter()
            .find(|(name, _, _)| name == callee)
            .cloned()
        {
            if self.call_depth >= MAX_CALL_DEPTH {
                self.aborted = true;
                return Value::Null;
            }
            let args = split_args(args_src);
            for (idx, param) in params.iter().enumerate() {
                let val = args
                    .get(idx)
                    .map(|a| self.eval_expr(doc, a))
                    .unwrap_or(Value::Null);
                self.set_var(param, val);
            }
            let prev_return = self.return_value.take();
            self.call_depth += 1;
            self.eval_script(doc, &body);
            self.call_depth = self.call_depth.saturating_sub(1);
            let result = self.return_value.take().unwrap_or(Value::Null);
            self.return_value = prev_return;
            return result;
        }

        Value::Null
    }
}

fn extract_callback_body(handler_raw: &str) -> String {
    if let Some((_, rhs)) = handler_raw.split_once("=>") {
        let rhs = rhs.trim();
        if let Some((_, block)) = split_brace_block(rhs) {
            return block;
        }
        return rhs.to_string();
    }
    if let Some((_, block)) = split_brace_block(handler_raw) {
        block
    } else if is_identifier(handler_raw) {
        format!("{handler_raw}()")
    } else {
        handler_raw.to_string()
    }
}

fn extract_single_param_callback(handler_raw: &str) -> (String, String) {
    if let Some((lhs, rhs)) = handler_raw.split_once("=>") {
        let param = lhs
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        let rhs = rhs.trim();
        let body = if let Some((_, block)) = split_brace_block(rhs) {
            block
        } else {
            rhs.to_string()
        };
        return (param, body);
    }
    if let Some((header, block)) = split_brace_block(handler_raw) {
        let param = header
            .find('(')
            .and_then(|open| {
                header[open + 1..]
                    .find(')')
                    .map(|close| &header[open + 1..open + 1 + close])
            })
            .and_then(|params| params.split(',').next())
            .unwrap_or("")
            .trim()
            .to_string();
        return (param, block);
    }
    (String::new(), handler_raw.to_string())
}

fn is_allowed_script_fetch(page_origin: Option<&str>, target_url: &str) -> bool {
    let target = target_url.trim();
    if target.is_empty() {
        return false;
    }
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("file:")
        || lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.starts_with("vbscript:")
        || lower.starts_with("flux:")
    {
        return false;
    }
    let Some(origin) = page_origin else {
        // Relative paths or same-host mock URLs allowed when no remote origin is bound
        return !lower.contains("://");
    };
    if !lower.contains("://") {
        return !target.contains("..");
    }
    extract_origin_tuple(origin) == extract_origin_tuple(target)
}

fn extract_origin_tuple(url: &str) -> Option<(String, String, u16)> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => 80u16,
        "https" => 443u16,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    if let Some((host, port_str)) = authority.rsplit_once(':') {
        let port = port_str.parse::<u16>().ok()?;
        Some((scheme, host.to_ascii_lowercase(), port))
    } else {
        Some((scheme, authority.to_ascii_lowercase(), default_port))
    }
}

fn values_equal(a: &Value, b: &Value, doc: &Document) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => (x - y).abs() <= f64::EPSILON,
        (Value::Element(x), Value::Element(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        _ => a.as_string(doc) == b.as_string(doc),
    }
}

fn update_inline_style(doc: &mut Document, id: NodeId, prop: &str, value: &str) {
    let existing = doc.get(id).data().attr("style").unwrap_or("").to_string();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for part in existing.split(';') {
        let Some((k, v)) = part.split_once(':') else {
            continue;
        };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim().to_string();
        if !k.is_empty() && !v.is_empty() {
            pairs.push((k, v));
        }
    }
    if let Some((_, slot)) = pairs.iter_mut().find(|(k, _)| k == prop) {
        *slot = value.to_string();
    } else {
        pairs.push((prop.to_string(), value.to_string()));
    }
    let updated = pairs
        .into_iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("; ");
    doc.set_attribute(id, "style", &updated);
}

fn camel_to_kebab(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_uppercase() {
            out.push('-');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn strip_comments(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    let mut in_quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        if let Some(q) = in_quote {
            out.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            } else if ch == q {
                in_quote = None;
            }
            continue;
        }
        if ch == '"' || ch == '\'' || ch == '`' {
            in_quote = Some(ch);
            out.push(ch);
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(inner) = chars.next() {
                if inner == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            out.push(' ');
            continue;
        }
        out.push(ch);
    }
    out
}

fn split_statements(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut paren = 0_i32;
    let mut brace = 0_i32;
    let mut quote: Option<char> = None;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            current.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => {
                quote = Some(ch);
                current.push(ch);
            }
            '(' => {
                paren += 1;
                current.push(ch);
            }
            ')' => {
                paren = (paren - 1).max(0);
                current.push(ch);
            }
            '{' => {
                brace += 1;
                current.push(ch);
            }
            '}' => {
                brace = (brace - 1).max(0);
                current.push(ch);
                if brace == 0 && paren == 0 {
                    let mut look = chars.clone();
                    while let Some(&c) = look.peek() {
                        if c.is_whitespace() {
                            look.next();
                        } else {
                            break;
                        }
                    }
                    let rest: String = look.take(4).collect();
                    if rest != "else" {
                        let trimmed = current.trim().to_string();
                        if !trimmed.is_empty() {
                            out.push(trimmed);
                        }
                        current.clear();
                    }
                }
            }
            ';' | '\n' if paren == 0 && brace == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    out.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        out.push(trimmed);
    }
    out
}

fn split_brace_block(input: &str) -> Option<(String, String)> {
    let open = input.find('{')?;
    let close = input.rfind('}')?;
    if close <= open {
        return None;
    }
    Some((
        input[..open].trim().to_string(),
        input[open + 1..close].trim().to_string(),
    ))
}

fn extract_paren_group(input: &str) -> Option<(String, &str)> {
    let bytes = input.as_bytes();
    if bytes.first() != Some(&b'(') {
        return None;
    }
    let mut depth = 0_i32;
    let mut quote: Option<u8> = None;
    let mut idx = 0;
    while idx < bytes.len() {
        let b = bytes[idx];
        if let Some(q) = quote {
            if b == b'\\' {
                idx += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            idx += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                return Some((input[1..idx].to_string(), &input[idx + 1..]));
            }
        }
        idx += 1;
    }
    None
}

fn extract_first_brace_block(input: &str) -> Option<(String, &str)> {
    let trimmed = input.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth = 0_i32;
    let mut quote: Option<u8> = None;
    let mut idx = 0;
    while idx < bytes.len() {
        let b = bytes[idx];
        if let Some(q) = quote {
            if b == b'\\' {
                idx += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            idx += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some((trimmed[1..idx].to_string(), &trimmed[idx + 1..]));
            }
        }
        idx += 1;
    }
    None
}

fn split_assignment(input: &str) -> Option<(&str, &str)> {
    let bytes = input.as_bytes();
    let mut paren = 0_i32;
    let mut brace = 0_i32;
    let mut quote: Option<u8> = None;
    let mut idx = 0;
    while idx < bytes.len() {
        let b = bytes[idx];
        if let Some(q) = quote {
            if b == b'\\' {
                idx += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            idx += 1;
            continue;
        }
        match b {
            b'"' | b'\'' => quote = Some(b),
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'{' => brace += 1,
            b'}' => brace = (brace - 1).max(0),
            b'=' if paren == 0 && brace == 0 => {
                let prev = if idx > 0 { bytes[idx - 1] } else { 0 };
                let next = bytes.get(idx + 1).copied().unwrap_or(0);
                if next != b'=' && !matches!(prev, b'=' | b'!' | b'<' | b'>') {
                    return Some((&input[..idx], &input[idx + 1..]));
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn split_top_level_op<'a>(input: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    let bytes = input.as_bytes();
    let op_bytes = op.as_bytes();
    let mut paren = 0_i32;
    let mut brace = 0_i32;
    let mut quote: Option<u8> = None;
    let mut last_match = None;
    let mut idx = 0;

    while idx < bytes.len() {
        let b = bytes[idx];
        if let Some(q) = quote {
            if b == b'\\' {
                idx += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            idx += 1;
            continue;
        }
        match b {
            b'"' | b'\'' => quote = Some(b),
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'{' => brace += 1,
            b'}' => brace = (brace - 1).max(0),
            _ if paren == 0 && brace == 0 && bytes[idx..].starts_with(op_bytes) => {
                if (op == "<" || op == ">") && bytes.get(idx + 1) == Some(&b'=') {
                    idx += 1;
                    continue;
                }
                if (op == "==" || op == "!=") && bytes.get(idx + 2) == Some(&b'=') {
                    idx += 1;
                    continue;
                }
                last_match = Some(idx);
                idx += op_bytes.len();
                continue;
            }
            _ => {}
        }
        idx += 1;
    }
    let pos = last_match?;
    Some((&input[..pos], &input[pos + op_bytes.len()..]))
}

fn find_call_open_paren(expr: &str) -> Option<usize> {
    let bytes = expr.as_bytes();
    let mut depth = 0_i32;
    let mut quote: Option<u8> = None;
    for idx in (0..bytes.len()).rev() {
        let b = bytes[idx];
        if let Some(q) = quote {
            if b == q && (idx == 0 || bytes[idx - 1] != b'\\') {
                quote = None;
            }
            continue;
        }
        match b {
            b'"' | b'\'' => quote = Some(b),
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    return (idx > 0).then_some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_args(args_src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut paren = 0_i32;
    let mut brace = 0_i32;
    let mut bracket = 0_i32;
    let mut quote: Option<char> = None;
    let mut chars = args_src.chars().peekable();

    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            current.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => {
                quote = Some(ch);
                current.push(ch);
            }
            '(' => {
                paren += 1;
                current.push(ch);
            }
            ')' => {
                paren = (paren - 1).max(0);
                current.push(ch);
            }
            '{' => {
                brace += 1;
                current.push(ch);
            }
            '}' => {
                brace = (brace - 1).max(0);
                current.push(ch);
            }
            '[' => {
                bracket += 1;
                current.push(ch);
            }
            ']' => {
                bracket = (bracket - 1).max(0);
                current.push(ch);
            }
            ',' if paren == 0 && brace == 0 && bracket == 0 => {
                out.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        out.push(trimmed);
    }
    out
}

fn parse_string_literal(expr: &str) -> Option<String> {
    let bytes = expr.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let quote = bytes[0];
    if (quote != b'"' && quote != b'\'') || bytes[bytes.len() - 1] != quote {
        return None;
    }
    let inner = &expr[1..expr.len() - 1];
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(ch);
        }
    }
    Some(out)
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_dom::parse_html;

    #[test]
    fn executes_script_mutations_and_click_listeners() {
        let mut doc = parse_html(
            "<title>Start</title>
             <h1 id=\"heading\">Before</h1>
             <p class=\"status\">Idle</p>
             <script>
               let count = 2 + 3;
               document.title = 'Count: ' + count;
               let h = document.getElementById('heading');
               h.textContent = 'Ready';
               h.style.color = '#e8c37a';
               let status = document.querySelector('.status');
               h.addEventListener('click', function() {
                 status.textContent = 'Clicked!';
               });
             </script>",
        );
        let mut runtime = ScriptRuntime::new();
        runtime.run_document_scripts(&mut doc);

        assert_eq!(doc.title(), "Count: 5");
        let h = doc.get_element_by_id("heading").unwrap();
        let status = doc.query_selector(".status").unwrap();
        assert_eq!(doc.text_content(h), "Ready");
        assert_eq!(doc.get(h).data().attr("style"), Some("color: #e8c37a"));
        assert_eq!(doc.text_content(status), "Idle");

        assert!(runtime.dispatch_click(&mut doc, h));
        assert_eq!(doc.text_content(status), "Clicked!");
    }

    #[test]
    fn parses_and_stringifies_json_in_scripts() {
        let mut doc = parse_html(
            "<p id=\"out\"></p>
             <script>
               let data = JSON.parse('{\"label\": \"Flux JSON\", \"code\": 3}');
               let out = document.getElementById('out');
               out.textContent = data.label + ' v' + data.code;
             </script>",
        );
        let mut runtime = ScriptRuntime::new();
        runtime.run_document_scripts(&mut doc);
        let out = doc.get_element_by_id("out").unwrap();
        assert_eq!(doc.text_content(out), "Flux JSON v3");
    }

    #[test]
    fn executes_for_loops_arrays_inputs_class_list_timers_and_fetch() {
        let mut doc = parse_html(
            "<input id=\"field\" type=\"text\" value=\"initial\">
             <input id=\"flag\" type=\"checkbox\">
             <div id=\"box\" class=\"card\"></div>
             <script>
               let nums = [10, 20, 30];
               let sum = 0;
               for (let i = 0; i < nums.length; i = i + 1) {
                 sum = sum + nums[i];
               }
               let f = document.getElementById('field');
               f.value = 'Sum: ' + sum;
               let c = document.getElementById('flag');
               c.checked = true;
               let box = document.getElementById('box');
               box.classList.add('active');
               let resp = fetch('/api/status');
               let info = resp.json();
               setTimeout(function() {
                 box.textContent = info.state + ':' + f.value;
               }, 10);
             </script>",
        );
        let mut runtime = ScriptRuntime::new();
        runtime.register_fetch_response("/api/status", 200, "{\"state\": \"online\"}");
        runtime.run_document_scripts(&mut doc);

        let field = doc.get_element_by_id("field").unwrap();
        let flag = doc.get_element_by_id("flag").unwrap();
        let box_el = doc.get_element_by_id("box").unwrap();
        assert_eq!(doc.input_value(field), "Sum: 60");
        assert!(doc.is_checked(flag));
        assert!(doc.get(box_el).data().has_class("active"));
        assert_eq!(doc.text_content(box_el), "online:Sum: 60");
    }

    #[test]
    fn aborts_infinite_recursion_and_enforces_same_origin_fetch() {
        let mut doc = parse_html(
            "<p id=\"status\">Safe</p>
             <script>
               function recurse() {
                 recurse();
               }
               recurse();
             </script>",
        );
        let mut runtime = ScriptRuntime::new();
        runtime.run_document_scripts(&mut doc);
        assert!(runtime.was_aborted());

        let mut sop_doc = parse_html(
            "<p id=\"out\"></p>
             <script>
               let blocked = fetch('https://other-origin.example/secret');
               let allowed = fetch('https://app.example/api');
               let out = document.getElementById('out');
               out.textContent = blocked.status + ':' + allowed.status;
             </script>",
        );
        let mut sop_rt = ScriptRuntime::new();
        sop_rt.set_page_origin("https://app.example/index.html");
        sop_rt.register_fetch_response("https://app.example/api", 200, "{\"ok\":true}");
        sop_rt.run_document_scripts(&mut sop_doc);
        let out = sop_doc.get_element_by_id("out").unwrap();
        assert_eq!(sop_doc.text_content(out), "403:200");
    }

    #[test]
    fn executes_modern_js_features_and_jit_toggle() {
        let script_html = "<div id=\"root\"><span class=\"chip\">A</span><span class=\"chip\">B</span></div>
             <p id=\"result\"></p>
             <script>
               const add = (a, b) => a + b;
               function computeLabel(base) {
                 let total = add(base, 8);
                 total += 2;
                 return `Total=${total}`;
               }
               let chips = document.querySelectorAll('.chip');
               let names = [];
               chips.forEach((el) => {
                 names.push(el.textContent);
               });
               let root = document.getElementById('root');
               let cached = document.getElementById('root');
               let res = document.getElementById('result');
               let mode = total_check();
               function total_check() {
                 return 5 > 3 ? 'ok' : 'bad';
               }
               console.log('computed', names.join('+'));
               res.textContent = computeLabel(10) + ':' + names.join(',') + ':' + mode + ':' + Math.max(4, 9);
             </script>";

        let mut doc_jit = parse_html(script_html);
        let mut rt_jit = ScriptRuntime::with_jit_enabled(true);
        rt_jit.run_document_scripts(&mut doc_jit);
        let res_jit = doc_jit.get_element_by_id("result").unwrap();
        assert_eq!(doc_jit.text_content(res_jit), "Total=20:A,B:ok:9");
        assert!(rt_jit.jit_stats().jit_enabled);
        assert!(rt_jit.jit_stats().compiled_blocks > 0);
        assert!(rt_jit.jit_stats().inline_cache_hits > 0);
        assert_eq!(rt_jit.console_logs(), &["computed A+B"]);

        let mut doc_interp = parse_html(script_html);
        let mut rt_interp = ScriptRuntime::with_jit_enabled(false);
        rt_interp.run_document_scripts(&mut doc_interp);
        let res_interp = doc_interp.get_element_by_id("result").unwrap();
        assert_eq!(doc_interp.text_content(res_interp), "Total=20:A,B:ok:9");
        assert!(!rt_interp.jit_stats().jit_enabled);
        assert_eq!(rt_interp.jit_stats().compiled_blocks, 0);
        assert_eq!(rt_interp.jit_stats().inline_cache_hits, 0);
    }
}
