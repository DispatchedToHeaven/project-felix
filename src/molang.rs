//! A small Molang: the expressions Blockbench writes into Bedrock animations.
//!
//! Numbers, `+ - * /`, comparisons, `&& || !`, `?:` and `??`, parentheses, `math.*`
//! functions (angles in degrees, as Bedrock's), `query.*` / `q.*` read from the
//! [`Context`], `variable.*` / `v.*` and `temp.*` / `t.*` read and assigned, statements
//! separated by `;`, and `return`. Anything unknown reads as 0, as Bedrock does.
//! No mod API types here.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f32),
    Query(String),
    Var(String),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Assign(String, Box<Expr>),
    Return(Box<Expr>),
    Block(Vec<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
    Coalesce,
}

/// What an expression reads: queries come from the game, variables live per entity.
pub trait Context {
    fn query(&self, name: &str) -> f32;
    fn var(&self, name: &str) -> f32;
    fn set_var(&mut self, name: &str, value: f32);
    /// `math.random` and friends: a number in 0..1.
    fn random(&mut self) -> f32 {
        0.5
    }
}

/// Queries in a map and variables in another: enough for tests and for the client.
#[derive(Default, Debug, Clone)]
pub struct MapContext {
    pub queries: HashMap<String, f32>,
    pub vars: HashMap<String, f32>,
    pub seed: u32,
}

impl Context for MapContext {
    fn query(&self, name: &str) -> f32 {
        self.queries.get(name).copied().unwrap_or(0.0)
    }
    fn var(&self, name: &str) -> f32 {
        self.vars.get(name).copied().unwrap_or(0.0)
    }
    fn set_var(&mut self, name: &str, value: f32) {
        self.vars.insert(name.to_string(), value);
    }
    fn random(&mut self) -> f32 {
        // xorshift: deterministic per context, good enough for jitter
        let mut x = self.seed.wrapping_add(0x9E37_79B9);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Parses an expression or a `;`-separated program.
pub fn parse(src: &str) -> Result<Expr, String> {
    let tokens = lex(src)?;
    let mut p = Parser { t: tokens, i: 0 };
    let mut stmts = Vec::new();
    while p.peek().is_some() {
        if p.eat(&Tok::Semi) {
            continue;
        }
        stmts.push(p.statement()?);
        if p.peek().is_some() && !p.eat(&Tok::Semi) {
            return Err(format!("unexpected {:?} in {src:?}", p.peek().unwrap()));
        }
    }
    Ok(match stmts.len() {
        0 => Expr::Num(0.0),
        1 if !matches!(stmts[0], Expr::Assign(..)) => stmts.pop().unwrap(),
        _ => Expr::Block(stmts),
    })
}

/// The value: a program's `return`, else its last expression's value.
pub fn eval(e: &Expr, cx: &mut dyn Context) -> f32 {
    match run(e, cx) {
        Flow::Value(v) | Flow::Return(v) => v,
    }
}

enum Flow {
    Value(f32),
    Return(f32),
}

fn run(e: &Expr, cx: &mut dyn Context) -> Flow {
    match e {
        Expr::Return(x) => Flow::Return(val(x, cx)),
        Expr::Block(list) => {
            let mut last = 0.0;
            for s in list {
                match run(s, cx) {
                    Flow::Return(v) => return Flow::Return(v),
                    Flow::Value(v) => last = v,
                }
            }
            Flow::Value(last)
        }
        _ => Flow::Value(val(e, cx)),
    }
}

fn truth(x: f32) -> bool {
    x != 0.0
}
fn b(x: bool) -> f32 {
    if x { 1.0 } else { 0.0 }
}

fn val(e: &Expr, cx: &mut dyn Context) -> f32 {
    match e {
        Expr::Num(n) => *n,
        Expr::Query(q) => cx.query(q),
        Expr::Var(v) => cx.var(v),
        Expr::Neg(x) => -val(x, cx),
        Expr::Not(x) => b(!truth(val(x, cx))),
        Expr::Bin(op, l, r) => {
            match op {
                Op::And => return b(truth(val(l, cx)) && truth(val(r, cx))),
                Op::Or => return b(truth(val(l, cx)) || truth(val(r, cx))),
                Op::Coalesce => {
                    // `v.x ?? 1`: the left side if it was ever set; a plain 0 here.
                    let a = val(l, cx);
                    return if a != 0.0 { a } else { val(r, cx) };
                }
                _ => {}
            }
            let (a, c) = (val(l, cx), val(r, cx));
            match op {
                Op::Add => a + c,
                Op::Sub => a - c,
                Op::Mul => a * c,
                Op::Div => {
                    if c == 0.0 {
                        0.0
                    } else {
                        a / c
                    }
                }
                Op::Lt => b(a < c),
                Op::Le => b(a <= c),
                Op::Gt => b(a > c),
                Op::Ge => b(a >= c),
                Op::Eq => b(a == c),
                Op::Ne => b(a != c),
                _ => unreachable!(),
            }
        }
        Expr::Cond(c, t, f) => {
            if truth(val(c, cx)) {
                val(t, cx)
            } else {
                val(f, cx)
            }
        }
        Expr::Assign(name, x) => {
            let v = val(x, cx);
            cx.set_var(name, v);
            v
        }
        Expr::Return(x) => val(x, cx),
        Expr::Block(_) => eval(e, cx),
        Expr::Call(name, args) => {
            let a: Vec<f32> = args.iter().map(|x| val(x, cx)).collect();
            let g = |i: usize| a.get(i).copied().unwrap_or(0.0);
            match name.as_str() {
                "abs" => g(0).abs(),
                "sin" => g(0).to_radians().sin(),
                "cos" => g(0).to_radians().cos(),
                "asin" => g(0).asin().to_degrees(),
                "acos" => g(0).acos().to_degrees(),
                "atan" => g(0).atan().to_degrees(),
                "atan2" => g(0).atan2(g(1)).to_degrees(),
                "sqrt" => g(0).max(0.0).sqrt(),
                "pow" => g(0).powf(g(1)),
                "exp" => g(0).exp(),
                "ln" => g(0).ln(),
                "floor" => g(0).floor(),
                "ceil" => g(0).ceil(),
                "round" => g(0).round(),
                "trunc" => g(0).trunc(),
                "mod" => {
                    if g(1) == 0.0 {
                        0.0
                    } else {
                        g(0) % g(1)
                    }
                }
                "min" => g(0).min(g(1)),
                "max" => g(0).max(g(1)),
                "clamp" => g(0).max(g(1)).min(g(2)),
                "lerp" => g(0) + (g(1) - g(0)) * g(2),
                "lerprotate" => {
                    let d = (g(1) - g(0) + 180.0).rem_euclid(360.0) - 180.0;
                    g(0) + d * g(2)
                }
                "hermite_blend" => {
                    let t = g(0);
                    3.0 * t * t - 2.0 * t * t * t
                }
                "pi" => std::f32::consts::PI,
                "sign" => {
                    if g(0) > 0.0 {
                        1.0
                    } else if g(0) < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
                "random" => {
                    let r = cx.random();
                    g(0) + (g(1) - g(0)) * r
                }
                "random_integer" => {
                    let r = cx.random();
                    (g(0) + (g(1) - g(0) + 1.0) * r).floor().min(g(1))
                }
                "die_roll" => {
                    let mut s = 0.0;
                    for _ in 0..g(0).max(0.0) as i32 {
                        s += g(1) + (g(2) - g(1)) * cx.random();
                    }
                    s
                }
                _ => 0.0,
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f32),
    Name(String),
    Sym(&'static str),
    Semi,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let s = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i] as char;
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && s.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < s.len() && ((s[i] as char).is_ascii_digit() || s[i] == b'.') {
                i += 1;
            }
            let n: f32 = src[start..i].parse().map_err(|_| format!("bad number {}", &src[start..i]))?;
            if i < s.len() && (s[i] == b'f' || s[i] == b'F') {
                i += 1;
            }
            out.push(Tok::Num(n));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < s.len() && ((s[i] as char).is_ascii_alphanumeric() || s[i] == b'_' || s[i] == b'.') {
                i += 1;
            }
            out.push(Tok::Name(src[start..i].to_ascii_lowercase()));
        } else if c == ';' {
            out.push(Tok::Semi);
            i += 1;
        } else {
            const SYMS: [&str; 18] =
                ["&&", "||", "<=", ">=", "==", "!=", "??", "+", "-", "*", "/", "<", ">", "!", "?", ":", "(", ")"];
            let rest = &src[i..];
            let sym = SYMS
                .iter()
                .find(|x| rest.starts_with(**x))
                .copied()
                .or_else(|| (rest.starts_with(',')).then_some(","))
                .or_else(|| (rest.starts_with('=')).then_some("="));
            let Some(sym) = sym else { return Err(format!("unexpected {c:?} in {src:?}")) };
            out.push(Tok::Sym(sym));
            i += sym.len();
        }
    }
    Ok(out)
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn sym(&mut self, s: &'static str) -> bool {
        self.eat(&Tok::Sym(s))
    }
    fn statement(&mut self) -> Result<Expr, String> {
        if let Some(Tok::Name(n)) = self.peek().cloned() {
            if n == "return" {
                self.i += 1;
                return Ok(Expr::Return(Box::new(self.expr()?)));
            }
            if let Some(var) = var_name(&n) {
                if self.t.get(self.i + 1) == Some(&Tok::Sym("=")) {
                    self.i += 2;
                    return Ok(Expr::Assign(var, Box::new(self.expr()?)));
                }
            }
        }
        self.expr()
    }
    fn expr(&mut self) -> Result<Expr, String> {
        let c = self.binary(0)?;
        if self.sym("?") {
            let t = self.expr()?;
            // `a ? b` alone means "b if a, else 0"
            let f = if self.sym(":") { self.expr()? } else { Expr::Num(0.0) };
            return Ok(Expr::Cond(Box::new(c), Box::new(t), Box::new(f)));
        }
        Ok(c)
    }
    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        const LEVELS: [&[(&str, Op)]; 6] = [
            &[("??", Op::Coalesce)],
            &[("||", Op::Or)],
            &[("&&", Op::And)],
            &[("==", Op::Eq), ("!=", Op::Ne), ("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt)],
            &[("+", Op::Add), ("-", Op::Sub)],
            &[("*", Op::Mul), ("/", Op::Div)],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut l = self.binary(level + 1)?;
        'outer: loop {
            for (s, op) in LEVELS[level] {
                if self.sym(s) {
                    let r = self.binary(level + 1)?;
                    l = Expr::Bin(*op, Box::new(l), Box::new(r));
                    continue 'outer;
                }
            }
            return Ok(l);
        }
    }
    fn unary(&mut self) -> Result<Expr, String> {
        if self.sym("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.sym("+") {
            return self.unary();
        }
        if self.sym("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.atom()
    }
    fn atom(&mut self) -> Result<Expr, String> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Sym("(")) => {
                self.i += 1;
                let e = self.expr()?;
                if !self.sym(")") {
                    return Err("missing )".into());
                }
                Ok(e)
            }
            Some(Tok::Name(n)) => {
                self.i += 1;
                if let Some(f) = n.strip_prefix("math.") {
                    if f == "pi" {
                        return Ok(Expr::Num(std::f32::consts::PI));
                    }
                    let mut args = Vec::new();
                    if self.sym("(") && !self.sym(")") {
                        loop {
                            args.push(self.expr()?);
                            if self.sym(")") {
                                break;
                            }
                            if !self.sym(",") {
                                return Err(format!("expected , or ) in math.{f}"));
                            }
                        }
                    }
                    return Ok(Expr::Call(f.to_string(), args));
                }
                if let Some(q) = n.strip_prefix("query.").or_else(|| n.strip_prefix("q.")) {
                    // `q.foo(1)`: arguments are read and dropped.
                    if self.sym("(") {
                        let mut depth = 1;
                        while depth > 0 {
                            match self.peek() {
                                Some(Tok::Sym("(")) => depth += 1,
                                Some(Tok::Sym(")")) => depth -= 1,
                                None => return Err("missing )".into()),
                                _ => {}
                            }
                            self.i += 1;
                        }
                    }
                    return Ok(Expr::Query(q.to_string()));
                }
                if let Some(v) = var_name(&n) {
                    return Ok(Expr::Var(v));
                }
                match n.as_str() {
                    "true" => Ok(Expr::Num(1.0)),
                    "false" => Ok(Expr::Num(0.0)),
                    "this" => Ok(Expr::Num(0.0)),
                    _ => Ok(Expr::Num(0.0)),
                }
            }
            other => Err(format!("unexpected {other:?}")),
        }
    }
}

/// `variable.x` / `v.x` → `v.x`, `temp.x` / `t.x` → `t.x`, `context.x` / `c.x` → `c.x`.
fn var_name(n: &str) -> Option<String> {
    for (long, short) in [("variable.", "v."), ("temp.", "t."), ("context.", "c.")] {
        if let Some(rest) = n.strip_prefix(long).or_else(|| n.strip_prefix(short)) {
            return Some(format!("{short}{rest}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, cx: &mut MapContext) -> f32 {
        eval(&parse(src).unwrap(), cx)
    }

    #[test]
    fn arithmetic_and_precedence() {
        let mut cx = MapContext::default();
        assert_eq!(run("1 + 2 * 3", &mut cx), 7.0);
        assert_eq!(run("(1 + 2) * 3", &mut cx), 9.0);
        assert_eq!(run("-2 * -3", &mut cx), 6.0);
        assert_eq!(run("10 / 4", &mut cx), 2.5);
        assert_eq!(run("1 > 0 && 2 < 1 ? 5 : 6", &mut cx), 6.0);
        assert_eq!(run("1 / 0", &mut cx), 0.0);
    }

    #[test]
    fn math_in_degrees_and_queries() {
        let mut cx = MapContext::default();
        cx.queries.insert("anim_time".into(), 0.25);
        let v = run("math.sin(query.anim_time * 360) * 10", &mut cx);
        assert!((v - 10.0).abs() < 1e-4, "{v}");
        assert!((run("math.cos(Q.Anim_Time * 720)", &mut cx) + 1.0).abs() < 1e-4);
        assert_eq!(run("math.clamp(5, 0, 2)", &mut cx), 2.0);
        assert_eq!(run("math.lerp(0, 10, 0.5)", &mut cx), 5.0);
        assert_eq!(run("q.unknown_thing(1, 2) + 1", &mut cx), 1.0);
    }

    #[test]
    fn variables_and_programs() {
        let mut cx = MapContext::default();
        assert_eq!(run("v.speed = 3; t.x = v.speed * 2; return t.x + 1;", &mut cx), 7.0);
        assert_eq!(run("variable.speed", &mut cx), 3.0);
        assert_eq!(run("v.unset ?? 4", &mut cx), 4.0);
    }

    #[test]
    fn errors() {
        assert!(parse("1 +").is_err());
        assert!(parse("(1").is_err());
        assert!(parse("1 $ 2").is_err());
    }
}
