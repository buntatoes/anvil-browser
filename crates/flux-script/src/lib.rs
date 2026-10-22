//! Page scripting runtime and DOM bindings.
//!
//! Executes inline `<script>` blocks and `click` event listeners against a
//! [`Document`].

use flux_dom::{Document, NodeId};
use flux_json::{JsonValue, parse_json, to_json_string};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Element(NodeId),
    Json(JsonValue),
}

impl Value {
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(b) => *b,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::Str(s) => !s.is_empty(),
            Self::Element(_) | Self::Json(_) => true,
        }
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
            Self::Json(j) => j.clone(),
        }
    }

    fn from_json_value(j: JsonValue) -> Self {
        match j {
            JsonValue::Null => Self::Null,
            JsonValue::Bool(b) => Self::Bool(b),
            JsonValue::Number(n) => Self::Number(n),
            JsonValue::String(s) => Self::Str(s),
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptRuntime {
    vars: Vec<(String, Value)>,
    functions: Vec<(String, Vec<String>, String)>,
    click_listeners: Vec<(NodeId, String)>,
    timers: Vec<(u64, String)>,
    fetch_mocks: Vec<(String, u16, String)>,
}

impl ScriptRuntime {
    pub fn new() -> Self {
        Self::default()
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
        self.collect_onclick_attrs(doc, doc.root());
        for script in doc.scripts() {
            self.eval_script(doc, &script);
        }
        let _ = self.run_timers(doc);
    }

    pub fn dispatch_click(&mut self, doc: &mut Document, target: NodeId) -> bool {
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
        let cleaned = strip_comments(source);
        let statements = split_statements(&cleaned);
        for stmt in statements {
            self.exec_statement(doc, &stmt);
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
        let stmt = raw.trim();
        if stmt.is_empty() {
            return;
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
                            if !self.eval_expr(doc, &cond_str).is_truthy() {
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

        if let Some((lhs, rhs)) = split_assignment(stripped) {
            let value = self.eval_expr(doc, rhs);
            self.assign_target(doc, lhs.trim(), value);
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
                    }
                    "className" => {
                        let text = value.as_string(doc);
                        doc.set_attribute(id, "class", &text);
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
        let expr = raw.trim();
        if expr.is_empty() {
            return Value::Null;
        }

        if expr.starts_with('[') && expr.ends_with(']') {
            let inner = &expr[1..expr.len() - 1];
            let items: Vec<JsonValue> = split_args(inner)
                .into_iter()
                .map(|item| self.eval_expr(doc, &item).to_json_value(doc))
                .collect();
            return Value::Json(JsonValue::Array(items));
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
                    if let Value::Json(JsonValue::Array(items)) = target {
                        if let Some(i) = idx_val.as_number() {
                            return items
                                .get(i.max(0.0) as usize)
                                .cloned()
                                .map(Value::from_json_value)
                                .unwrap_or(Value::Null);
                        }
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

    fn eval_call(&mut self, doc: &mut Document, callee: &str, args_src: &str) -> Value {
        if callee == "setTimeout" {
            let args = split_args(args_src);
            if let Some(first) = args.first() {
                let handler_raw = first.trim();
                let body = if let Some((_, block)) = split_brace_block(handler_raw) {
                    block
                } else if is_identifier(handler_raw) {
                    format!("{handler_raw}()")
                } else {
                    handler_raw.to_string()
                };
                let delay = args
                    .get(1)
                    .and_then(|d| self.eval_expr(doc, d).as_number())
                    .unwrap_or(0.0)
                    .max(0.0) as u64;
                self.timers.push((delay, body));
            }
            return Value::Number(self.timers.len() as f64);
        }

        if callee == "fetch" {
            let args = split_args(args_src);
            let url = args
                .first()
                .map(|a| self.eval_expr(doc, a).as_string(doc))
                .unwrap_or_default();
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

            if obj_expr == "document" {
                let args = split_args(args_src);
                let first = args
                    .first()
                    .map(|a| self.eval_expr(doc, a).as_string(doc))
                    .unwrap_or_default();
                return match method {
                    "getElementById" => doc
                        .get_element_by_id(&first)
                        .map(Value::Element)
                        .unwrap_or(Value::Null),
                    "querySelector" => doc
                        .query_selector(&first)
                        .map(Value::Element)
                        .unwrap_or(Value::Null),
                    "createElement" => {
                        let id = doc.create_element(&first, Vec::new());
                        Value::Element(id)
                    }
                    _ => Value::Null,
                };
            }

            match self.eval_expr(doc, obj_expr) {
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
                        "appendChild" if !args.is_empty() => {
                            if let Value::Element(child_id) = self.eval_expr(doc, &args[0]) {
                                let _ = doc.append(id, child_id);
                                return Value::Element(child_id);
                            }
                            return Value::Null;
                        }
                        "addEventListener" if args.len() >= 2 => {
                            let event = self.eval_expr(doc, &args[0]).as_string(doc);
                            if event.eq_ignore_ascii_case("click") {
                                let handler_raw = args[1].trim();
                                let body = if let Some((_, block)) = split_brace_block(handler_raw)
                                {
                                    block
                                } else if is_identifier(handler_raw) {
                                    format!("{handler_raw}()")
                                } else {
                                    handler_raw.to_string()
                                };
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
            let args = split_args(args_src);
            for (idx, param) in params.iter().enumerate() {
                let val = args
                    .get(idx)
                    .map(|a| self.eval_expr(doc, a))
                    .unwrap_or(Value::Null);
                self.set_var(param, val);
            }
            self.eval_script(doc, &body);
        }

        Value::Null
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
        if ch == '"' || ch == '\'' {
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
            '"' | '\'' => {
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
            '"' | '\'' => {
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
}
