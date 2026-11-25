//! Tests of the parser against the rules of Section 3 of SPEC.md.
//! Trees are compared in the text form of `koo_syntax::dump`.

use koo_syntax::ast::{ItemKind, StmtKind};
use koo_syntax::{dump, parse_file, parse_fragment};

/// Parses statements and returns their dumps, one per line.
#[track_caller]
fn stmts(src: &str) -> String {
    let parsed = parse_fragment(src);
    assert!(
        parsed.diagnostics.is_empty(),
        "unexpected errors in {src:?}: {:#?}",
        parsed.diagnostics
    );
    parsed.file.items.iter().map(dump::item).collect::<Vec<_>>().join("\n")
}

/// Parses one expression.
#[track_caller]
fn expr(src: &str) -> String {
    let parsed = parse_fragment(&format!("it := {src}"));
    assert!(
        parsed.diagnostics.is_empty(),
        "unexpected errors in {src:?}: {:#?}",
        parsed.diagnostics
    );
    let ItemKind::Stmt(stmt) = &parsed.file.items[0].kind else {
        panic!("not a statement")
    };
    let StmtKind::Let { value, .. } = &stmt.kind else {
        panic!("not a binding")
    };
    dump::expr(value)
}

/// Parses a complete file and returns its dump.
#[track_caller]
fn file(src: &str) -> String {
    let parsed = parse_file(src);
    assert!(
        parsed.diagnostics.is_empty(),
        "unexpected errors in {src:?}: {:#?}",
        parsed.diagnostics
    );
    dump::file(&parsed.file).trim_end().to_string()
}

fn fragment_errors(src: &str) -> Vec<String> {
    parse_fragment(src).diagnostics.into_iter().map(|d| d.message).collect()
}

fn file_errors(src: &str) -> Vec<String> {
    parse_file(src).diagnostics.into_iter().map(|d| d.message).collect()
}

/// Asserts that parsing `src` as a fragment reports an error containing `needle`.
#[track_caller]
fn rejects(src: &str, needle: &str) {
    let errors = fragment_errors(src);
    assert!(
        errors.iter().any(|e| e.contains(needle)),
        "expected an error containing {needle:?} for {src:?}, got {errors:#?}"
    );
}

#[track_caller]
fn file_rejects(src: &str, needle: &str) {
    let errors = file_errors(src);
    assert!(
        errors.iter().any(|e| e.contains(needle)),
        "expected an error containing {needle:?} for {src:?}, got {errors:#?}"
    );
}

// ---- Section 3.4: precedence ------------------------------------------------------------

#[test]
fn precedence_levels() {
    assert_eq!(expr("a + b * c"), "(+ a (* b c))");
    assert_eq!(expr("a * b + c"), "(+ (* a b) c)");
    assert_eq!(expr("a - b - c"), "(- (- a b) c)");
    assert_eq!(expr("a / b % c"), "(% (/ a b) c)");
    assert_eq!(expr("-a * b"), "(* (- a) b)");
    assert_eq!(expr("-x.f"), "(- (. x f))");
    assert_eq!(expr("-f(x)?"), "(- (? (call f x)))");
    assert_eq!(expr("a..b + 1"), "(.. a (+ b 1))");
    assert_eq!(expr("0..=n == r"), "(== (..= 0 n) r)");
    assert_eq!(expr("a + 1 < b * 2"), "(< (+ a 1) (* b 2))");
    assert_eq!(expr("not a == b"), "(not (== a b))");
    assert_eq!(expr("not a and not b"), "(and (not a) (not b))");
    assert_eq!(expr("a or b and c"), "(or a (and b c))");
    assert_eq!(expr("not a == b and c or d"), "(or (and (not (== a b)) c) d)");
    assert_eq!(expr("x is Cash and ready"), "(and (is x Cash) ready)");
    assert_eq!(expr("a or b else c"), "(else (or a b) c)");
}

#[test]
fn else_is_right_associative_and_lowest() {
    assert_eq!(expr("a else b else c"), "(else a (else b c))");
    assert_eq!(expr("(hits[key] else 0) + 1"), "(+ (else (index hits key) 0) 1)");
    assert_eq!(expr("int.parse(s) else 0"), "(else (call (. int parse) s) 0)");
}

#[test]
fn comparisons_and_ranges_do_not_chain() {
    rejects("x := a < b < c", "comparisons do not chain");
    rejects("x := a == b != c", "comparisons do not chain");
    rejects("x := a..b..c", "ranges do not chain");
    assert_eq!(expr("(a < b) == c"), "(== (< a b) c)");
}

#[test]
fn postfix_chains() {
    assert_eq!(expr("find_user(e)?.name"), "(. (? (call find_user e)) name)");
    assert_eq!(expr("xs[0][1]"), "(index (index xs 0) 1)");
    assert_eq!(
        expr("out[0..int.from(n)?]"),
        "(index out (.. 0 (? (call (. int from) n))))"
    );
    assert_eq!(expr("limiter.send.reset()"), "(call (. (. limiter send) reset))");
    assert_eq!(
        expr("http.App.new().get(path)"),
        "(call (. (call (. (. http App) new)) get) path)"
    );
}

#[test]
fn reserved_words_are_member_names_after_a_dot() {
    assert_eq!(expr("event.type"), "(. event type)");
    assert_eq!(expr("rows.all()"), "(call (. rows all))");
    assert_eq!(stmts("log.error(\"failed\", e)"), "(call (. log error) \"failed\" e)");
    assert_eq!(expr("row.int(\"amount\")?"), "(? (call (. row int) \"amount\"))");
    assert_eq!(expr("x.none"), "(. x none)");
}

// ---- Section 3.3: literals --------------------------------------------------------------

#[test]
fn literals() {
    assert_eq!(expr("[1, 2, 3]"), "[1 2 3]");
    assert_eq!(expr("[]"), "[]");
    assert_eq!(expr("[\"a\": 1, \"b\": 2]"), "[\"a\": 1 \"b\": 2]");
    assert_eq!(expr("[:]"), "[:]");
    assert_eq!(expr("(1, \"a\")"), "(tuple 1 \"a\")");
    assert_eq!(expr("()"), "()");
    assert_eq!(expr("(a)"), "a");
    assert_eq!(expr("19.99d * 2d"), "(* 19.99d 2d)");
    assert_eq!(expr("1h30min + 250ms"), "(+ 1h30min 250ms)");
    assert_eq!(expr("[\n    1,\n    2,\n]"), "[1 2]");
    assert_eq!(expr("`a\\b`"), "(raw \"a\\\\b\")");
    assert_eq!(expr("none"), "none");
}

#[test]
fn interpolated_strings() {
    assert_eq!(expr("\"total: {order.total}\""), "(str \"total: \" (. order total))");
    assert_eq!(
        expr("\"{ratio:.2} of {name:>10}\""),
        "(str (fmt ratio \".2\") \" of \" (fmt name \">10\"))"
    );
    assert_eq!(expr("\"{a + b}{f(x, y: 1)}\""), "(str (+ a b) (call f x y: 1))");
    assert_eq!(expr("\"a \\{literal} brace\""), "\"a {literal} brace\"");
    assert_eq!(expr("\"{names[\"first\"]}\""), "(str (index names \"first\"))");
    rejects("x := \"{a +}\"", "expected an expression");
    rejects("x := \"{a b}\"", "expected `}` or `:` and a format");
}

#[test]
fn text_blocks() {
    let src = "rows := db.query(\"\"\"\n    select *\n    from orders\n    where id = {id}\n    \"\"\")?";
    assert_eq!(
        stmts(src),
        "(let rows (? (call (. db query) (str \"select *\\nfrom orders\\nwhere id = \" id))))"
    );
}

#[test]
fn struct_literals() {
    assert_eq!(expr("Order { id: id, total: 10d }"), "(new Order id: id total: 10d)");
    assert_eq!(expr("Order { id, total }"), "(new Order id total)");
    assert_eq!(
        expr("Payment.Card { last4: \"4242\", brand: \"visa\" }"),
        "(new (. Payment Card) last4: \"4242\" brand: \"visa\")"
    );
    assert_eq!(
        expr("http.Response { status: 200 }"),
        "(new (. http Response) status: 200)"
    );
    assert_eq!(
        expr("Reply { user: Reply.User { id, name }, items: [] }"),
        "(new Reply user: (new (. Reply User) id name) items: [])"
    );
    assert_eq!(
        expr("Cache<string, int> { entries: [:], max: 10 }"),
        "(new Cache<string, int> entries: [:] max: 10)"
    );
    assert_eq!(expr("Event { type: \"click\" }"), "(new Event type: \"click\")");
    assert_eq!(expr("Order { id }.describe()"), "(call (. (new Order id) describe))");
    assert_eq!(expr("Timeout"), "Timeout");
    rejects("x := Event { type }", "needs a value");
}

// ---- Section 3.9: disambiguation --------------------------------------------------------

#[test]
fn type_arguments_follow_capitalized_names() {
    assert_eq!(
        expr("Channel<int>.new(capacity: 8)"),
        "(call (. Channel<int> new) capacity: 8)"
    );
    assert_eq!(
        expr("Shared<Flags>.new(load_flags()?)"),
        "(call (. Shared<Flags> new) (? (call load_flags)))"
    );
    assert_eq!(expr("List<byte>.with_size(n)"), "(call (. List<byte> with_size) n)");
    assert_eq!(expr("Map<string, [int]>.new()"), "(call (. Map<string, [int]> new))");
    assert_eq!(
        expr("cache.Cache<string, Order>.new(max: 10)"),
        "(call (. (. cache Cache)<string, Order> new) max: 10)"
    );
    assert_eq!(expr("List<List<int>>.new()"), "(call (. List<List<int>> new))");
    assert_eq!(expr("a < b"), "(< a b)");
    assert_eq!(expr("(Low) < level"), "(< Low level)");
    rejects("x := Low < level", "expected a type");
    rejects("x := Low < Level > other", "after the type arguments");
    let help = parse_fragment("x := Low < level").diagnostics[0].help[0].clone();
    assert!(help.contains("parenthesize the left side"), "{help}");
}

#[test]
fn c_names_take_no_type_arguments_or_literals() {
    let src = "import c \"zlib.h\" link \"z\"\n\nfn f(rc: i32, n: i32) -> bool {\n    if rc != c.Z_OK { return false }\n    return c.Z_OK < n\n}\n";
    assert_eq!(
        file(src),
        "(import-c \"zlib.h\" link \"z\" as c)\n(fn f(rc: i32, n: i32) -> bool { (if (!= rc (. c Z_OK)) { (return false) }); (return (< (. c Z_OK) n)) })"
    );
    let aliased = "import c \"sqlite3.h\" as sqlite link \"sqlite3\"\n\nfn f(n: i32) -> bool {\n    return sqlite.SQLITE_OK < n\n}\n";
    assert!(parse_file(aliased).diagnostics.is_empty());
}

#[test]
fn lambdas() {
    assert_eq!(expr("x -> x + 1"), "(fn (x) (+ x 1))");
    assert_eq!(expr("(a, b) -> a + b"), "(fn (a, b) (+ a b))");
    assert_eq!(expr("() -> get_order(db, id)"), "(fn () (call get_order db id))");
    assert_eq!(
        expr("(e: NotFound) -> http.status(404)"),
        "(fn (e: NotFound) (call (. http status) 404))"
    );
    assert_eq!(expr("_ -> 0"), "(fn (_) 0)");
    assert_eq!(
        expr("xs.map(x -> x + 1).sum()"),
        "(call (. (call (. xs map) (fn (x) (+ x 1))) sum))"
    );
    assert_eq!(
        stmts("xs.each(x -> total += x)"),
        "(call (. xs each) (fn (x) (+= total x)))"
    );
    assert_eq!(
        expr("e -> fail Wrapped { cause: e }"),
        "(fn (e) (fail (new Wrapped cause: e)))"
    );
    assert_eq!(
        expr("(xs: var [int]) -> xs.push(1)"),
        "(fn (xs: var [int]) (call (. xs push) 1))"
    );
    assert_eq!(
        stmts(
            "app.get(\"/orders/:id\", req -> {\n    id := Id.parse(req.path(\"id\"))?\n    return http.json(get_order(db, id)?)\n})"
        ),
        "(call (. app get) \"/orders/:id\" (fn (req) { (let id (? (call (. Id parse) (call (. req path) \"id\")))); (return (call (. http json) (? (call get_order db id)))) }))"
    );
    assert_eq!(expr("(a, b)"), "(tuple a b)");
}

#[test]
fn struct_literals_in_conditions_are_parenthesized() {
    assert_eq!(
        stmts("if a == (Order { id }) { f() }"),
        "(if (== a (new Order id)) { (call f) })"
    );
    // Without parentheses the brace opens the block, and `id` is not a statement.
    rejects("if a == Order { id } { f() }", "not a statement");
    assert_eq!(stmts("if p is Cash { f() }"), "(if (is p Cash) { (call f) })");
    assert_eq!(stmts("for x in Items { f(x) }"), "(for x in Items { (call f x) })");
    // Inside parentheses or brackets the restriction is lifted.
    assert_eq!(
        stmts("if xs.contains(Order { id }) { f() }"),
        "(if (call (. xs contains) (new Order id)) { (call f) })"
    );
    assert_eq!(
        stmts("while q.push(Job { id }) { f() }"),
        "(while (call (. q push) (new Job id)) { (call f) })"
    );
}

#[test]
fn binding_else() {
    assert_eq!(
        stmts("user := load_user(id) else e => {\n    log.warn(\"failed\", e)\n    fail Unavailable { cause: e }\n}"),
        "(let user (else (call load_user id) e => { (call (. log warn) \"failed\" e); (fail (new Unavailable cause: e)) }))"
    );
    assert_eq!(
        stmts("row := db.query_one(q)? else fail NotFound { id }"),
        "(let row (else (? (call (. db query_one) q)) (fail (new NotFound id))))"
    );
    assert_eq!(stmts("done.recv() else return"), "(else (call (. done recv)) (return))");
    assert_eq!(stmts("x := next() else break"), "(let x (else (call next) (break)))");
    assert_eq!(
        expr("f() else e => default_for(e)"),
        "(else (call f) e => (call default_for e))"
    );
    assert_eq!(expr("a else if c { 1 } else { 2 }"), "(else a (if c { 1 } { 2 }))");
}

#[test]
fn labels() {
    assert_eq!(
        stmts("outer: for x in xs {\n    for y in ys {\n        if x == y { break outer }\n        continue\n    }\n}"),
        "(outer: for x in xs { (for y in ys { (if (== x y) { (break outer) }); (continue) }) })"
    );
    assert_eq!(
        stmts("retry: while pending() { continue retry }"),
        "(retry: while (call pending) { (continue retry) })"
    );
}

// ---- Section 3.7: statements and lines --------------------------------------------------

#[test]
fn a_newline_ends_a_statement_in_a_block() {
    assert_eq!(stmts("a := 1\nb := 2"), "(let a 1)\n(let b 2)");
    rejects("a := 1 b := 2", "expected the end of the line");
    rejects("f() g()", "expected the end of the line");
}

#[test]
fn a_leading_minus_starts_a_statement() {
    rejects("x := a\n-b", "not a statement");
    assert_eq!(stmts("x := (a\n    - b)"), "(let x (- a b))");
    assert_eq!(stmts("x := a - b"), "(let x (- a b))");
}

#[test]
fn lines_continue_on_a_leading_operator() {
    assert_eq!(stmts("x := a\n    + b\n    * c"), "(let x (+ a (* b c)))");
    assert_eq!(
        stmts("ok := ready\n    and not failed\n    or forced"),
        "(let ok (or (and ready (not failed)) forced))"
    );
    assert_eq!(stmts("same := left\n    == right"), "(let same (== left right))");
    assert_eq!(stmts("r := lo\n    ..hi"), "(let r (.. lo hi))");
    assert_eq!(
        stmts("total := xs\n    .filter(x -> x > 0)\n    .map(x -> x * 2)\n    .sum()"),
        "(let total (call (. (call (. (call (. xs filter) (fn (x) (> x 0))) map) (fn (x) (* x 2))) sum)))"
    );
    assert_eq!(
        stmts("port := env(\"PORT\")\n    else \"8080\""),
        "(let port (else (call env \"PORT\") \"8080\"))"
    );
}

#[test]
fn nothing_about_how_a_line_ends_continues_it() {
    rejects("x := a +\n    b", "expected an expression");
    rejects("x := f(a) and\n    g(b)", "expected an expression");
    rejects("x :=\n    5", "expected an expression");
}

#[test]
fn newlines_are_white_space_inside_parentheses_and_brackets() {
    assert_eq!(stmts("f(\n    a,\n    b,\n)"), "(call f a b)");
    assert_eq!(stmts("x := (a\n    - b\n    - c)"), "(let x (- (- a b) c))");
    assert_eq!(
        stmts("o := Order {\n    id,\n    total: 1d,\n}"),
        "(let o (new Order id total: 1d))"
    );
    assert_eq!(stmts("y := xs[\n    0\n]"), "(let y (index xs 0))");
}

#[test]
fn a_block_inside_parentheses_has_significant_newlines_again() {
    assert_eq!(
        stmts("run(() -> {\n    a()\n    b()\n})"),
        "(call run (fn () { (call a); (call b) }))"
    );
    rejects("run(() -> {\n    a() b()\n})", "expected the end of the line");
    rejects("run(() -> {\n    x := 1\n    -x\n})", "not a statement");
}

#[test]
fn a_call_does_not_continue_onto_the_next_line() {
    // `(a, b) := pair` on its own line is a binding, not a call of the line before.
    assert_eq!(stmts("f()\n(a, b) := pair"), "(call f)\n(let (a, b) pair)");
    assert_eq!(stmts("x := y\n[1, 2].each(f)"), "(let x y)\n(call (. [1 2] each) f)");
}

#[test]
fn the_opening_brace_is_on_the_same_line() {
    file_rejects("fn f()\n{\n}", "a function needs a body");
    rejects("if ready\n{\n    f()\n}", "expected `{`");
    assert_eq!(
        stmts("if ready {\n    f()\n}\nelse {\n    g()\n}"),
        "(if ready { (call f) } { (call g) })"
    );
}

#[test]
fn expression_statements_are_calls() {
    assert_eq!(stmts("f()"), "(call f)");
    assert_eq!(stmts("file.close()?"), "(? (call (. file close)))");
    rejects("x + 1", "not a statement");
    rejects("x", "not a statement");
    rejects("\"text\"", "not a statement");
    rejects("x == 1", "not a statement");
}

#[test]
fn there_are_no_semicolons() {
    rejects("f(); g()", "expected the end of the line, found `;`");
    let parsed = parse_fragment("f();");
    assert!(advice(&parsed.diagnostics[0]).contains("no semicolons"));
    assert!(parsed.diagnostics[0].fix.is_some(), "a `;` that ends a line is removed");
}

// ---- Sections 3.5, 6.7: bindings, assignments, control flow -----------------------------

#[test]
fn bindings() {
    assert_eq!(stmts("x := 1"), "(let x 1)");
    assert_eq!(stmts("var x := 1"), "(var x 1)");
    assert_eq!(stmts("x: int? = largest(xs)"), "(let x: int? (call largest xs))");
    assert_eq!(stmts("var hits: [string: int] = [:]"), "(var hits: [string: int] [:])");
    assert_eq!(stmts("(a, b) := pair"), "(let (a, b) pair)");
    assert_eq!(stmts("(a, _) := pair"), "(let (a, _) pair)");
    assert_eq!(stmts("var ((a, b), c) := nested"), "(var ((a, b), c) nested)");
    assert_eq!(
        stmts("(user, orders) := all(get_user(id), get_orders(id))?"),
        "(let (user, orders) (? (call all (call get_user id) (call get_orders id))))"
    );
    assert_eq!(stmts("_ := f()"), "(let _ (call f))");
    assert_eq!(
        stmts("r: Order ! NotFound = get_order(db, id)"),
        "(let r: Order ! NotFound (call get_order db id))"
    );
    assert_eq!(stmts("q: Payment = Cash"), "(let q: Payment Cash)");
    rejects("x: int", "a binding is always given a value");
    rejects("(a, b): Pair = p", "not a statement");
}

#[test]
fn assignments() {
    assert_eq!(stmts("x = 1"), "(= x 1)");
    assert_eq!(stmts("hits[key] = n"), "(= (index hits key) n)");
    assert_eq!(stmts("p.x += 1"), "(+= (. p x) 1)");
    assert_eq!(
        stmts("total -= row.int(\"amount\")?"),
        "(-= total (? (call (. row int) \"amount\")))"
    );
    assert_eq!(stmts("a.b[i].c *= 2"), "(*= (. (index (. a b) i) c) 2)");
    assert_eq!(stmts("x /= 2"), "(/= x 2)");
    rejects("f() = 1", "cannot be assigned to");
    rejects("a + b = 1", "cannot be assigned to");
    rejects("x := y = 1", "expected the end of the line");
}

#[test]
fn conditionals_and_loops() {
    assert_eq!(
        stmts("if a { f() } else if b { g() } else { h() }"),
        "(if a { (call f) } (if b { (call g) } { (call h) }))"
    );
    assert_eq!(
        stmts("if user := find_user(email) { greet(user) }"),
        "(if (bind user (call find_user email)) { (call greet user) })"
    );
    assert_eq!(
        stmts("while row := rows.next()? {\n    total += row.int(\"amount\")?\n}"),
        "(while (bind row (? (call (. rows next)))) { (+= total (? (call (. row int) \"amount\"))) })"
    );
    assert_eq!(
        stmts("for (k, v) in m { f(k, v) }"),
        "(for (k, v) in m { (call f k v) })"
    );
    assert_eq!(stmts("for _ in 0..8 { f() }"), "(for _ in (.. 0 8) { (call f) })");
    assert_eq!(
        stmts("for x in take xs { keep(x) }"),
        "(for x in take xs { (call keep x) })"
    );
    assert_eq!(stmts("return"), "(return)");
}

#[test]
fn if_and_match_used_for_a_value() {
    assert_eq!(expr("if a > b { a } else { b }"), "(if (> a b) { a } { b })");
    assert_eq!(
        expr("if a { 1 } else if b { 2 } else { 3 }"),
        "(if a { 1 } (if b { 2 } { 3 }))"
    );
    assert_eq!(
        stmts("x := if ok { v } else {\n    log.warn(\"no\")\n    return\n}"),
        "(let x (if ok { v } { (call (. log warn) \"no\"); (return) }))"
    );
    assert_eq!(stmts("return if a { 1 } else { 2 }"), "(return (if a { 1 } { 2 }))");
    // A block of several statements has no value, so a bare expression in it is an error.
    rejects("x := if ok { f()\n v } else { w }", "not a statement");
    // Used as a statement, the branches are ordinary blocks.
    rejects("if ok { v } else { w }", "not a statement");
    assert_eq!(
        expr("match n {\n    0 => \"zero\"\n    _ => \"many\"\n}"),
        "(match n (0 => \"zero\") (_ => \"many\"))"
    );
}

#[test]
fn blocks_statements() {
    assert_eq!(
        stmts("defer { gauge.add(-1) }"),
        "(defer { (call (. gauge add) (- 1)) })"
    );
    assert_eq!(
        stmts("scope {\n    spawn { work(1) }\n    spawn { work(2) }\n}"),
        "(scope { (spawn { (call work 1) }); (spawn { (call work 2) }) })"
    );
    assert_eq!(
        stmts("unsafe {\n    // the pointer is valid\n    c_free(p)\n}"),
        "(unsafe { (call c_free p) })"
    );
    rejects(
        "fn f() {\n    fn g() {}\n}",
        "a declaration cannot appear inside a block",
    );
}

#[test]
fn select_statement() {
    let src = "select {\n    msg := inbox.recv()     => handle(msg)\n    outbox.send(pending)    => pending = next()\n    _ := shutdown.recv()    => break\n    after 30s               => heartbeat()\n}";
    assert_eq!(
        stmts(src),
        "(select (msg := (call (. inbox recv)) => (call handle msg)) ((call (. outbox send) pending) => (= pending (call next))) (_ := (call (. shutdown recv)) => (break)) (after 30s => (call heartbeat)))"
    );
}

// ---- Section 6.6: patterns --------------------------------------------------------------

#[test]
fn patterns() {
    let src = "match payment {\n    Card { brand: \"amex\", last4 }               => amex_fee(last4)\n    Card { .. }                                  => card_fee()\n    Transfer { iban } if iban.starts_with(\"DE\")  => sepa()\n    Transfer | Cash                              => no_fee()\n}";
    assert_eq!(
        stmts(src),
        "(match payment (Card {brand: \"amex\", last4} => (call amex_fee last4)) (Card {..} => (call card_fee)) (Transfer {iban} if (call (. iban starts_with) \"DE\") => (call sepa)) (Transfer | Cash => (call no_fee)))"
    );
    let src = "match get_order(db, id) {\n    order            => respond(order)\n    ! NotFound       => respond_404()\n    ! e: sql.Error   => {\n        log.error(\"db failure\", e)\n        respond_503()\n    }\n}";
    assert_eq!(
        stmts(src),
        "(match (call get_order db id) (order => (call respond order)) (! NotFound => (call respond_404)) (! e: sql.Error => { (call (. log error) \"db failure\" e); (call respond_503) }))"
    );
    let src = "match v {\n    none => a()\n    (0, y) => b(y)\n    1..=9 => c()\n    -1 => d()\n    'a'..'z' => e()\n    \"x\" | \"y\" => f()\n    w: Writer => g(w)\n    ! sql.Error | NotFound { id, .. } => h()\n    Payment.Cash => i()\n    Point { x: 0, y: (a, b) } => j()\n    _ => k()\n}";
    assert_eq!(
        stmts(src),
        "(match v (none => (call a)) ((0, y) => (call b y)) (1..=9 => (call c)) ((- 1) => (call d)) ('a'..'z' => (call e)) (\"x\" | \"y\" => (call f)) (w: Writer => (call g w)) (! sql.Error | NotFound {id, ..} => (call h)) (Payment.Cash => (call i)) (Point {x: 0, y: (a, b)} => (call j)) (_ => (call k)))"
    );
    rejects("match v {\n    P { .., x } => f()\n}", "`..` comes last");
    rejects("match v {\n    a => f() b => g()\n}", "one per line");
    assert_eq!(
        stmts("match v {\n    Transfer\n    | Cash => no_fee()\n    x => pending = x\n}"),
        "(match v (Transfer | Cash => (call no_fee)) (x => (= pending x)))"
    );
}

// ---- Declarations -----------------------------------------------------------------------

#[test]
fn imports() {
    assert_eq!(
        file(
            "import std/http\nimport std/sql/postgres\nimport pkg.koo.io/acme/stripe v2\nimport ./orders\nimport ../shared/util as u\nimport pkg.koo.io/acme/orders-service v1 as orders_service\n"
        ),
        "(import std/http as http)\n(import std/sql/postgres as postgres)\n(import pkg.koo.io/acme/stripe v2 as stripe)\n(import ./orders as orders)\n(import ../shared/util as u)\n(import pkg.koo.io/acme/orders-service v1 as orders_service)"
    );
    file_rejects("import pkg.koo.io/acme/stripe\n", "major version");
    file_rejects("import std/http v2\n", "only a registry package");
    file_rejects("import http\n", "not a package path");
    file_rejects("import pkg.koo.io/acme/orders-service v1\n", "cannot be used as a name");
    file_rejects("fn main() {}\nimport std/http\n", "imports come before");
    file_rejects("import std/http as Http\n", "capitalized");
}

#[test]
fn type_declarations() {
    assert_eq!(file("pub type Id = distinct int"), "pub (type Id distinct int)");
    assert_eq!(file("type UserId = Id"), "(type UserId Id)");
    assert_eq!(
        file(
            "pub type Order = {\n    id: Id,\n    note: string = \"\",       // default\n    _cache: [string: int] = [:],\n}"
        ),
        "pub (type Order {id: Id, note: string = \"\", _cache: [string: int] = [:]})"
    );
    assert_eq!(
        file("pub type Cache<K: Hashable, V> = { entries: [K: V], max: int }"),
        "pub (type Cache<K: Hashable, V> {entries: [K: V], max: int})"
    );
    assert_eq!(
        file("pub type Postgres: Database = { pool: Pool }"),
        "pub (type Postgres: Database {pool: Pool})"
    );
    assert_eq!(
        file("pub type Reply = { user: { id: Id, name: string }, items: [{ sku: string, qty: int }] }"),
        "pub (type Reply {user: {id: Id, name: string}, items: [{sku: string, qty: int}]})"
    );
    assert_eq!(
        file("pub resource File = { _fd: i32 }"),
        "pub (resource File {_fd: i32})"
    );
    assert_eq!(
        file("type Event = { type: string, created: Time @json(\"createdAt\") }"),
        "(type Event {type: string, created: Time @json})"
    );
    assert_eq!(
        file("type Handler = fn(Request) -> Response ! Failure"),
        "(type Handler fn(Request) -> Response ! Failure)"
    );
    file_rejects("type order = { id: Id }", "lowercase name");
    file_rejects("type Http_Server = { port: int }", "not UpperCamelCase");
    file_rejects("type T = { userId: int }", "not snake_case");
    file_rejects(
        "resource File = distinct i32",
        "a resource is declared with a struct type",
    );
    file_rejects("fn f(x: { a: int }) {}", "a struct type cannot be written here");
}

#[test]
fn types() {
    let check = |src: &str| {
        let text = format!("type T = {src}");
        assert_eq!(file(&text), format!("(type T {src})"));
    };
    check("[int]");
    check("[string: [Order]]");
    check("[byte; 16]");
    check("(int, string)");
    check("()");
    check("int??");
    check("Set<Id>");
    check("sql.Pool");
    check("Cache<string, List<Order>>");
    check("Reply.User");
    check("fn()");
    check("fn(A, B) -> R ! E");
    check("fn(var [int], take Order)");
    check("resource fn() -> T ! E | Timeout");
    check("[Order ! NotFound]");
    check("Row? ! sql.Error");
    check("(Order ! NotFound)?");
    check("(fn() -> int)?");
    check("(fn() -> int) ! Closed");
    check("Channel<Order ! NotFound | sql.Error>");
    assert_eq!(file("type T = fn() -> int ! Closed"), "(type T fn() -> int ! Closed)");
    assert_eq!(stmts("var xs: List<int>= []"), "(var xs: List<int> [])");
    file_rejects("type T = order", "expected a type, found `order`");
}

#[test]
fn enums_and_errors() {
    assert_eq!(
        file(
            "pub enum Payment {\n    Card { last4: string, brand: string }\n    Transfer { iban: string }\n    Cash\n}"
        ),
        "pub (enum Payment [Card {last4: string, brand: string} Transfer {iban: string} Cash])"
    );
    assert_eq!(
        file("enum Tree { Leaf { value: int }, Node { left: Tree, right: Tree } }"),
        "(enum Tree [Leaf {value: int} Node {left: Tree, right: Tree}])"
    );
    assert_eq!(file("enum Level { Low, High }"), "(enum Level [Low High])");
    assert_eq!(
        file("@closed\npub enum Json {\n    Null\n    Bool { value: bool }\n}"),
        "@closed pub (enum Json [Null Bool {value: bool}])"
    );
    assert_eq!(
        file("enum Field<T> {\n    Missing\n    Value { value: T }\n}"),
        "(enum Field<T> [Missing Value {value: T}])"
    );
    assert_eq!(
        file(
            "error NotFound { id: Id }\nerror Conflict\nerror StorageError { cause: Failure }\nerror OrderError = NotFound | sql.Error"
        ),
        "(error NotFound {id: Id})\n(error Conflict {})\n(error StorageError {cause: Failure})\n(error OrderError = NotFound | sql.Error)"
    );
    file_rejects("enum E { A B }", "after the variant");
    file_rejects("enum E { a }", "lowercase name");
}

#[test]
fn functions() {
    assert_eq!(
        file("fn get_order(db: sql.Pool, id: Id) -> Order ! NotFound | sql.Error { return find(db, id)? }"),
        "(fn get_order(db: sql.Pool, id: Id) -> Order ! NotFound | sql.Error { (return (? (call find db id))) })"
    );
    assert_eq!(
        file("fn log_all(level: int, messages: ...string) {}"),
        "(fn log_all(level: int, messages: ...string) {})"
    );
    assert_eq!(
        file("fn Order.new(customer: Id) -> Order {}"),
        "(fn Order.new(customer: Id) -> Order {})"
    );
    assert_eq!(
        file("fn (self: Order) describe() -> string {}"),
        "(fn (self: Order) describe() -> string {})"
    );
    assert_eq!(
        file("fn (self: var Order) apply(pct: Decimal) {}"),
        "(fn (self: var Order) apply(pct: Decimal) {})"
    );
    assert_eq!(
        file("fn (self: take Order) into_invoice() -> Invoice {}"),
        "(fn (self: take Order) into_invoice() -> Invoice {})"
    );
    assert_eq!(
        file("fn (self: Cache<K, V>) get(key: K) -> V? {}"),
        "(fn (self: Cache<K, V>) get(key: K) -> V? {})"
    );
    assert_eq!(
        file("fn Cache<K, V>.new(max: int) -> Cache<K, V> {}"),
        "(fn Cache<K, V>.new(max: int) -> Cache<K, V> {})"
    );
    assert_eq!(
        file("fn main() ! sql.Error | http.Error {}"),
        "(fn main() ! sql.Error | http.Error {})"
    );
    assert_eq!(
        file("fn connect(host: string, port: int = 5432) {}"),
        "(fn connect(host: string, port: int = 5432) {})"
    );
    assert_eq!(
        file("fn sort<T: Ordered>(xs: take [T]) -> [T] {}"),
        "(fn sort<T: Ordered>(xs: take [T]) -> [T] {})"
    );
    assert_eq!(file("fn helper() -> int ! {}"), "(fn helper() -> int ! {})");
    assert_eq!(
        file("fn (self: Request) json<T: json.Decode>() -> T ! json.Error {}"),
        "(fn (self: Request) json<T: json.Decode>() -> T ! json.Error {})"
    );
    assert_eq!(
        file("fn with_timeout<T: resource, E>(d: Duration, f: fn() -> T ! E) -> T ! E | Timeout {}"),
        "(fn with_timeout<T: resource, E>(d: Duration, f: fn() -> T ! E) -> T ! E | Timeout {})"
    );
    assert_eq!(
        file("fn isolate<T: resource, E>(f: take resource fn() -> T ! E) -> T ! E | Panic {}"),
        "(fn isolate<T: resource, E>(f: take resource fn() -> T ! E) -> T ! E | Panic {})"
    );
}

#[test]
fn a_long_error_set_wraps_with_leading_bars() {
    assert_eq!(
        file("fn fetch() -> Order ! NotFound\n    | sql.Error\n    | Timeout {}"),
        "(fn fetch() -> Order ! NotFound | sql.Error | Timeout {})"
    );
    // `!` does not continue a line, so the set begins on the line of the return type.
    file_rejects("fn fetch() -> Order\n    ! NotFound {}", "a function needs a body");
}

#[test]
fn function_declaration_errors() {
    file_rejects("fn GetOrder() {}", "only types have capitalized names");
    file_rejects("fn getOrder() {}", "not snake_case");
    file_rejects("fn f(a: ...int, b: int) {}", "must be the last one");
    file_rejects("fn f(int: string) {}", "is a primitive type");
    file_rejects("fn f()", "a function needs a body");
    file_rejects("fn (this: Order) f() {}", "expected `self`");
    file_rejects("fn (self) f() {}", "expected `:`");
    file_rejects("fn f(x) {}", "the parameter `x` needs a type");
    file_rejects("fn f(x int) {}", "expected `:` before the type of `x`");
    file_rejects(
        "fn f(int x, string y) {}",
        "the type of a parameter comes after its name",
    );
    file_rejects("fn f(): int {}", "the return type of a function follows `->`");
    file_rejects("fn f() int {}", "the return type of a function follows `->`");
    file_rejects("fn f() -> void {}", "declares no return type");
    file_rejects("fn f<>() {}", "empty type parameter list");
}

#[test]
fn reserved_words_and_primitive_names_may_name_members() {
    assert_eq!(
        file("pub fn error(message: log.Message) {}"),
        "pub (fn error(message: log.Message) {})"
    );
    assert_eq!(
        file("fn (self: Row) int(column: string) -> int ! Error {}"),
        "(fn (self: Row) int(column: string) -> int ! Error {})"
    );
    assert_eq!(
        file("fn int.parse(s: string) -> int ! ParseError {}"),
        "(fn int.parse(s: string) -> int ! ParseError {})"
    );
    assert_eq!(
        file("fn (self: Rows) all() -> [Row] ! Error {}"),
        "(fn (self: Rows) all() -> [Row] ! Error {})"
    );
    rejects("int := 3", "is a primitive type");
    rejects("for string in xs { f() }", "is a primitive type");
}

#[test]
fn interfaces() {
    assert_eq!(
        file(
            "pub interface Database {\n    fn query_one(q: sql.Query) -> sql.Row? ! sql.Error\n    fn query(q: sql.Query) -> sql.Rows ! sql.Error\n}"
        ),
        "pub (interface Database [(fn query_one(q: sql.Query) -> sql.Row? ! sql.Error) (fn query(q: sql.Query) -> sql.Rows ! sql.Error)])"
    );
    assert_eq!(
        file("pub resource interface Iterator<T> {\n    fn (self: var) next() -> T?\n}"),
        "pub (resource-interface Iterator<T> [(fn (self: var) next() -> T?)])"
    );
    assert_eq!(
        file(
            "interface Decode {\n    fn Self.from_json(bytes: take [byte]) -> Self ! Error\n    fn (self: take) finish()\n}"
        ),
        "(interface Decode [(fn Self.from_json(bytes: take [byte]) -> Self ! Error) (fn (self: take) finish())])"
    );
    file_rejects("interface I {\n    fn f() {}\n}", "has no body");
    file_rejects(
        "interface I {\n    fn (self: var Order) f()\n}",
        "a receiver in an interface has no type",
    );
    file_rejects("interface I {\n    x: int\n}", "an interface holds function signatures");
}

#[test]
fn actors() {
    let src = "@mailbox(4096)\npub actor RateLimiter {\n    var hits: [string: int] = [:]\n    limit: int\n\n    fn allow(key: string) -> bool {\n        n := (hits[key] else 0) + 1\n        hits[key] = n\n        return n <= limit\n    }\n\n    pub fn reset() {\n        hits = [:]\n    }\n}";
    assert_eq!(
        file(src),
        "@mailbox pub (actor RateLimiter {var hits: [string: int] = [:], limit: int} [(fn allow(key: string) -> bool { (let n (+ (else (index hits key) 0) 1)); (= (index hits key) n); (return (<= n limit)) }) pub (fn reset() { (= hits [:]) })])"
    );
    assert_eq!(
        file("actor Cache<K: Hashable, V> {\n    var entries: [K: V] = [:]\n}"),
        "(actor Cache<K: Hashable, V> {var entries: [K: V] = [:]} [])"
    );
    file_rejects(
        "actor A {\n    fn (self: A) f() {}\n}",
        "a method of an actor has no receiver",
    );
    file_rejects(
        "actor A {\n    fn f(x: take Job) {}\n}",
        "every parameter of an actor method is `take` already",
    );
}

#[test]
fn attributes() {
    let parsed = parse_file(
        "@supervise(restart(max: 5, within: 1min))\n@mailbox(64)\nactor A {\n}\n\n@c pub fn exported(n: i32) -> i32 { return n }\n\n@derive(yaml.codec)\n@json(rename: camel, strict)\ntype Config = { port: int @json(\"p\") @sql(\"port\") = 80 }\n",
    );
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let names: Vec<Vec<&str>> = parsed
        .file
        .items
        .iter()
        .map(|i| i.attrs.iter().map(|a| a.name.name.as_str()).collect())
        .collect();
    assert_eq!(
        names,
        vec![vec!["supervise", "mailbox"], vec!["c"], vec!["derive", "json"]]
    );
    assert_eq!(parsed.file.items[2].attrs[1].args.len(), 2);
}

#[test]
fn constants_and_documentation() {
    let parsed = parse_file(
        "/// The most retries.\n/// Per request.\npub max_retries := 3\n\n// not a doc comment\ntimeout: Duration = 30s\n",
    );
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    assert_eq!(parsed.file.items[0].docs, vec!["The most retries.", "Per request."]);
    assert!(parsed.file.items[0].public);
    assert!(parsed.file.items[1].docs.is_empty());
    assert_eq!(dump::item(&parsed.file.items[1]), "(const timeout: Duration 30s)");
    file_rejects("var counter := 0", "a top-level binding cannot be `var`");
    file_rejects("f()", "a statement cannot stand outside a function");
    file_rejects("x = 5", "a constant is declared with `:=`");
    file_rejects("\"text\"", "expected a declaration");
}

// ---- Recovery ---------------------------------------------------------------------------

#[test]
fn several_errors_are_reported_in_one_pass() {
    let src = "fn a() {\n    x := 1 +\n    f()\n    y := )\n}\n\nfn B() {}\n\nfn c() {\n    g(;)\n}\n";
    let parsed = parse_file(src);
    assert_eq!(parsed.diagnostics.len(), 4, "{:#?}", parsed.diagnostics);
    // The functions around the errors are still in the tree.
    assert_eq!(parsed.file.items.len(), 2);
}

#[test]
fn unclosed_brackets_do_not_hang() {
    for src in [
        "fn f() {",
        "fn f() { g(",
        "x := [1, 2",
        "type T = {",
        "match x {",
        "x := \"{a",
        "f(a, (b",
        "}",
        ")",
        "fn f() { ) }",
    ] {
        let _ = parse_file(src);
        let _ = parse_fragment(src);
    }
}

#[test]
fn diagnostics_render_like_the_specification() {
    let src = "fn main() {\n    order := get_order(db, id) extra\n}\n";
    let parsed = parse_file(src);
    let rendered = parsed.diagnostics[0].render("main.koo", src, &koo_syntax::LineIndex::new(src));
    assert_eq!(
        rendered,
        "error: expected the end of the line, found `extra`\n --> main.koo:2:32\n  |\n2 |     order := get_order(db, id) extra\n  |                                ^^^^^\n"
    );
}

// ---- Found by review --------------------------------------------------------------------

#[test]
fn range_patterns_have_literal_bounds() {
    rejects(
        "match v {\n    0..=max => g()\n}",
        "the bounds of a range pattern are literals",
    );
    rejects(
        "match v {\n    1.. => g()\n}",
        "the bounds of a range pattern are literals",
    );
}

#[test]
fn an_if_used_for_a_value_may_hold_another() {
    assert_eq!(
        expr("if a { if b { 1 } else { 2 } } else { 3 }"),
        "(if a { (if b { 1 } { 2 }) } { 3 })"
    );
    assert_eq!(
        expr("if a { match b {\n    0 => x\n    _ => y\n} } else { z }"),
        "(if a { (match b (0 => x) (_ => y)) } { z })"
    );
    // As a statement, the same `if` holds statements at every level.
    rejects("if a { if b { 1 } else { 2 } } else { f() }", "not a statement");
    assert_eq!(
        stmts("if a { if b { f() } else { g() } }"),
        "(if a { (if b { (call f) } { (call g) }) })"
    );
    rejects(
        "x := if a { f()\n if b { 1 } else { 2 } } else { 3 }",
        "not a statement",
    );
}

#[test]
fn if_and_match_statements_are_whole_statements() {
    rejects("match k {\n    A => save()\n}?", "expected the end of the line");
    rejects("if a { f() } else { g() }.h()", "expected the end of the line");
    assert_eq!(
        stmts("r := (match k {\n    A => save()\n})?"),
        "(let r (? (match k (A => (call save)))))"
    );
}

#[test]
fn c_imports_take_any_plain_string() {
    assert_eq!(
        file("import c `zlib.h` link `z`"),
        "(import-c \"zlib.h\" link \"z\" as c)"
    );
    file_rejects("import c \"{name}.h\"", "an interpolation cannot be used here");
}

#[test]
fn a_line_break_after_a_parenthesis_ends_the_statement() {
    rejects("h := (x, y)\n-> x", "expected an expression, found `->`");
    assert!(!fragment_errors("h := (x, y)\n-> x").is_empty());
    assert!(!fragment_errors("if (m, n)\n:= pair() { g() }").is_empty());
    assert_eq!(stmts("h := (x, y) -> x"), "(let h (fn (x, y) x))");
}

#[test]
fn only_c_names_are_lowercase_types() {
    file_rejects("fn a(x: sql.pool) {}", "expected a type name, found `pool`");
    file_rejects(
        "fn a(x: Any) -> bool { return x is foo.bar }",
        "expected a type name, found `bar`",
    );
    assert!(file_errors("import c \"o.h\"\n\nfn a(x: c.order, y: ffi.Ptr<c.order>) {}").is_empty());
    assert!(file_errors("import c \"o.h\" as o\n\nfn a(x: o.order) {}").is_empty());
    assert_eq!(
        file("import c \"o.h\"\n\ntype T = ffi.Ptr<c.order>"),
        "(import-c \"o.h\" as c)\n(type T ffi.Ptr<c.order>)"
    );
}

#[test]
fn attribute_strings_are_plain() {
    file_rejects("@export(\"a{b}\")\nfn a() {}", "cannot contain an interpolation");
    file_rejects("enum E {\n    @json(\"a\") A\n}", "expected a variant name");
}

#[test]
fn underscores_separate_digits() {
    for bad in ["0x_", "0b_", "0x_FF", "1_", "1__0", "1_.5", "1h_30min", "1_e5"] {
        assert!(!fragment_errors(&format!("x := {bad}")).is_empty(), "accepted {bad}");
    }
    for good in [
        "1_000",
        "0xFF_FF",
        "0b1010_1010",
        "1_000.000_1",
        "1_500ms",
        "19_999.99d",
    ] {
        assert!(fragment_errors(&format!("x := {good}")).is_empty(), "rejected {good}");
    }
}

#[test]
fn field_shorthand_needs_a_usable_name() {
    rejects("match e {\n    Event { type } => g()\n}", "needs a pattern");
    assert_eq!(
        stmts("match e {\n    Event { type: t } => g(t)\n}"),
        "(match e (Event {type: t} => (call g t)))"
    );
}

#[test]
fn import_paths_follow_the_grammar() {
    file_rejects("import ./../a", "not a package path");
    file_rejects("import ./_", "cannot be used as a name");
    assert!(file_errors("import ../../shared/util").is_empty());
}

#[test]
fn else_does_not_bind_before_an_arm_arrow() {
    assert_eq!(
        stmts("match k {\n    A if cache.get(k) else fallback => g()\n}"),
        "(match k (A if (else (call (. cache get) k) fallback) => (call g)))"
    );
    assert_eq!(
        stmts("select {\n    v := ch.recv() else dflt => g(v)\n}"),
        "(select (v := (else (call (. ch recv)) dflt) => (call g v)))"
    );
    // Parenthesized, or in the arm's body, `else` binds as usual.
    assert_eq!(
        stmts("match k {\n    A if (load(k) else e => recover(e)) => g()\n    B => load(k) else e => recover(e)\n}"),
        "(match k (A if (else (call load k) e => (call recover e)) => (call g)) (B => (else (call load k) e => (call recover e))))"
    );
}

#[test]
fn function_types_that_return_functions() {
    assert_eq!(
        file("type T = fn() -> fn() -> int ! E"),
        "(type T fn() -> (fn() -> int ! E))"
    );
    assert_eq!(
        file("type T = fn() -> (fn() -> int) ! E"),
        "(type T fn() -> (fn() -> int) ! E)"
    );
    assert_eq!(file("type T = fn() -> (int ! E)"), "(type T fn() -> (int ! E))");
    assert_eq!(file("fn f() -> fn() -> int ! E {}"), "(fn f() -> (fn() -> int ! E) {})");
    assert_eq!(
        file("fn f() -> (fn() -> int) ! E {}"),
        "(fn f() -> (fn() -> int) ! E {})"
    );
}

/// Everything a diagnostic says besides its message.
fn advice(diagnostic: &koo_syntax::Diagnostic) -> String {
    let mut text = diagnostic.help.join("\n");
    text.push_str(&diagnostic.notes.join("\n"));
    if let Some(fix) = &diagnostic.fix {
        text.push_str(&fix.message);
        for (_, edit) in &fix.edits {
            text.push_str(edit);
        }
    }
    text
}

#[test]
fn messages() {
    rejects("x := !y", "expected an expression, found `!`");
    assert!(advice(&parse_fragment("x := !y").diagnostics[0]).contains("not"));
    rejects("fn f(int: string) {}", "cannot be used as a parameter name");
    file_rejects("import std/http as Http", "expected an import name");
    rejects("x := \"\"\"\n    a\\\n    \"\"\"", "at the end of a line");
}
