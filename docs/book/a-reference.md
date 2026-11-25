# Appendix A: Reference Manual

## A.1 Introduction

This manual describes the Koo language as the compiler builds it at the time of writing. It is a summary for reference, and is meant to be exact where the chapters were meant to be clear. The language as it is intended to become is defined by its [specification](../../SPEC.md); where a construct of the specification is not listed here, the compiler does not build it yet, and Appendix C says what it does instead.

## A.2 Lexical Conventions

A program is one or more files of text in UTF-8. The text is divided into *tokens*: names, keywords, constants, operators and punctuation. Blanks, tabs and comments separate tokens and are otherwise ignored. The end of a line is significant, as A.2.6 describes.

### A.2.1 Comments

The characters `//` begin a comment, which ends at the end of the line. A comment that begins with `///` is a documentation comment, and belongs to the declaration that follows it. There are no comments that span lines, and a comment does not begin inside a string.

### A.2.2 Names

A name is a sequence of the letters `a` to `z` and `A` to `Z`, digits, and underscores, that does not begin with a digit. Case is significant, and it is prescribed:

- the name of a type, a type parameter, an interface, an error, an actor or a variant begins with an upper case letter;
- every other name, that of a variable, constant, function, method, parameter, field or package, begins with a lower case letter or an underscore, and contains no upper case letters.

Names that contain two underscores in a row are used by the standard library, and should be avoided.

### A.2.3 Keywords

The following names are reserved, and may not be used otherwise:

    actor      distinct   fn         is         resource   take
    and        else       for        match      return     type
    as         enum       if         not        scope      unsafe
    break      error      import     or         select     var
    continue   fail       in         pub        spawn      while
    defer      interface

`true`, `false` and `none` are constants. `self` is the name of the receiver in a method.

### A.2.4 Constants

An *integer constant* is a sequence of decimal digits; or `0x` followed by hexadecimal digits, of which the letters may be in either case; or `0b` followed by binary digits. Its type is `int`. A constant that does not fit in an `int` is an error.

A *floating constant* has digits, a decimal point and more digits; or digits and an exponent, which is `e` or `E`, an optional sign, and digits; or all of these. Its type is `float`.

Underscores may appear between the digits of either kind, and are ignored.

A *duration constant* is one or more groups of decimal digits and a unit, with nothing between them, the units being `h`, `min`, `s`, `ms`, `us` and `ns`, each at most once and in that order: `90s`, `1h30min`. Its type is `Duration`, and it is an error if it is longer than a `Duration` can be.

A *string constant* is one of three forms.

- Characters between double quotes, on one line. A backslash introduces an escape sequence: `\n` newline, `\r` carriage return, `\t` tab, `\0` the character of code zero, `\\` backslash, `\"` double quote, `\'` single quote, `\{` opening brace, and `\u{`*h*`}`, where *h* is one or more hexadecimal digits, the character with that code. An opening brace not preceded by a backslash begins an *interpolation* (A.5.9).
- A *raw string*: characters between backquotes, which may extend over several lines. No character in it is special.
- A *text block*: a line that ends with three double quotes, the lines of the text, and a line that holds three double quotes after only blanks. The blanks before the closing quotes are removed from the beginning of every line of the text, and the text does not include the final newline. Escape sequences and interpolations are as in the first form.

The type of a string constant is `string`.

### A.2.5 Operators and Punctuation

    +    -    *    /    %    ==   !=   <    <=   >    >=
    =    :=   +=   -=   *=   /=   ..   ..=  ->   =>   ?    !
    (    )    [    ]    {    }    ,    .    :    |    _

### A.2.6 Lines

A statement or declaration ends at the end of its line. The end of a line is ignored, and the line continues on the next:

- inside parentheses and square brackets, and inside the braces of a structure literal, a structure type or a pattern;
- when the next line begins with a period, with `else`, with `|`, or with one of the binary operators `*  /  %  +  ..  ..=  ==  !=  <  <=  >  >=  and  or`.

A line that begins with `-` is not a continuation. A list between parentheses, brackets or literal braces may end with a comma. The opening brace of a block is on the line of the construct that the block belongs to.

## A.3 Types

### A.3.1 Basic Types

| | |
| --- | --- |
| `int` | integers from −2<sup>63</sup> to 2<sup>63</sup> − 1 |
| `float` | IEEE 754 double-precision floating-point numbers |
| `bool` | `true` and `false` |
| `byte` | integers from 0 to 255 |
| `string` | sequences of characters, encoded in UTF-8, always valid |
| `Duration` | lengths of time, as a signed count of nanoseconds in 64 bits |
| `Time` | instants, as the nanoseconds from the beginning of 1970 in UTC, in 64 bits |

### A.3.2 Derived Types

| Written | |
| --- | --- |
| `[T]` | a *list* of values of type `T` |
| `[K: V]` | a *map* from keys of type `K` to values of type `V` |
| `Set<T>` | a *set* of values of type `T` |
| `(T1, T2, ...)` | a *tuple*, of two or more values |
| `T?` | an *optional*: a value of type `T`, or `none` |
| `T ! E1 \| E2 ...` | a *result*: a value of type `T`, or an error of one of the types listed |
| `fn(T1, T2, ...) -> R` | a *function* that takes arguments of the types listed and returns `R`; `-> R` is absent for a function that returns nothing, and `! E` may follow as in a result |

together with the types that a program declares: structures, enumerations, errors, and the handles of actors (A.7).

The key type of a map and the element type of a set must be hashable: `int`, `bool`, `byte`, `string`, and lists, tuples, optionals, structures and enumerations of hashable types. A `float` is not hashable.

### A.3.3 Values and Storage

Every type is a value type. Assigning a value, passing it as an argument, returning it, or storing it in another value makes the destination hold the value independently of the source: no later change to either is visible through the other. The one exception is the handle of an actor, a value that identifies the actor; copies of a handle identify the same actor.

A value is destroyed, and its storage is released, when the last variable or value that holds it is destroyed or is given another value.

### A.3.4 Conversions

No value is converted from one type to another implicitly, with one exception: a value of type `T` is accepted where a `T?` is required, and where a result `T ! E` is required. The conversions that are written are the functions and methods of A.9.

## A.4 Declarations at the Top Level

A file consists of imports followed by declarations: constants, functions, types, enumerations, errors, interfaces and actors. All the files of a package form one scope, and a declaration may be used anywhere in the package, before or after the place where it is written. No name may be declared twice at the top level; there is no overloading.

An *import*

    import std/io

makes the functions of a standard package available in the file that contains it, under the last part of the package's name: `io.read_line()`. `import std/io as files` gives the package another name in that file, `files.read_line()`. The packages are `std/io`, `std/os`, `std/time`, `std/http` and `std/test`. An import that its file does not use is an error.

A *constant*

    name := constant
    name: Type = constant

declares a name whose value is an integer, floating, string, boolean or duration constant, or the negation of a number. A constant cannot be `var`.

The other declarations are described in A.7.

## A.5 Expressions

The operators, in decreasing order of precedence:

| | Operators | Associativity |
| --- | --- | --- |
| 1 | `x.f`  `x.f(...)`  `f(...)`  `x[i]`  `x?` | left |
| 2 | `-x` | |
| 3 | `*`  `/`  `%` | left |
| 4 | `+`  `-` | left |
| 5 | `a..b`  `a..=b` | none |
| 6 | `==`  `!=`  `<`  `<=`  `>`  `>=` | none |
| 7 | `not x` | |
| 8 | `and` | left |
| 9 | `or` | left |
| 10 | `x else y` | right |

The operands of an operator, and the arguments of a call, are evaluated from left to right.

### A.5.1 Primary Expressions

A name; a constant; an expression in parentheses; a list `[e1, e2, ...]`; a map `[k1: v1, k2: v2, ...]`, the empty map being `[:]`; a tuple `(e1, e2, ...)`; a structure literal (A.7.2); a variant (A.7.3); an error value (A.7.4); a lambda (A.5.8); an `if` or `match` used for its value (A.5.7).

The element type of `[]`, the types of `[:]` and the type of `none` are taken from the context, which must determine them.

### A.5.2 Postfix Expressions

`x.f` is the field `f` of the structure or error `x`; or, for a list, string, map or set, the property `len`, the number of its elements, bytes, entries or members.

`f(args)` calls the function `f`; `x.m(args)` calls the method `m` with receiver `x`; `T.f(args)` calls the function `f` of the type `T`; `p.f(args)` calls the function `f` of the imported package `p`. An argument is an expression, or `name: expression`, which passes the argument to the parameter of that name; or `var place`, which is required, and only allowed, when the parameter is `var`. Arguments by position come before arguments by name. A parameter that has a default may be left without an argument.

`x[i]`, where `x` is a list and `i` an `int`, is element `i` of the list. If `i` is negative or not less than `x.len`, the program panics. Where `x` is a map, `x[k]` is an optional: the value for the key `k`, or `none`. A string cannot be subscripted.

`x?`, where `x` is a result, is the value of `x`; if `x` is an error, the function in which the expression appears fails with that error, which must be in the function's error set.

### A.5.3 Arithmetic Operators

The operands of `+`, `-`, `*`, `/` and `%` are both `int` or both `float`, and the result has the same type. The operand of unary `-` is an `int` or a `float`.

For `int`, `/` truncates toward zero, and `%` has the sign of its left operand; the program panics if the right operand of either is zero, or if the result of `+`, `-`, `*`, `/` or unary `-` cannot be represented.

For `float`, the operations are those of IEEE 754, and none panics.

`+` also applies to two strings, and yields their concatenation.

For durations and times the operators are these, and no others: `Duration + Duration`, `Duration - Duration` and unary `-` yield a `Duration`; `Duration * int`, `int * Duration` and `Duration / int` yield a `Duration`; `Time + Duration`, `Duration + Time` and `Time - Duration` yield a `Time`; `Time - Time` yields a `Duration`. The program panics if a result cannot be represented.

### A.5.4 Ranges

`a..b` and `a..=b`, where `a` and `b` are of type `int`, denote the integers from `a` up to but not including `b`, and up to and including `b`. A range may appear only as the sequence of a `for` statement, or, with constant bounds, as a pattern.

### A.5.5 Comparison Operators

`==` and `!=` take two operands of the same type and yield a `bool`; where one is an optional and the other a value of the type it may hold, the value is taken as an optional that holds it, so that `name == "ada"` is false for a `name` that is `none`. They are defined for every type except functions and types that contain one. Equality is structural: two values are equal when they are the same variant, or have equal fields, elements, or characters. Two maps are equal when they have the same keys with equal values, and two sets when they have the same members; the order in which keys or members were added plays no part. Two floating-point values are compared as IEEE 754 prescribes.

`<`, `<=`, `>` and `>=` are defined for `int`, `float`, `byte`, `string`, `Duration` and `Time`, and for a type that has a method `compare(other: T) -> int`, in which case `a < b` means `a.compare(b) < 0`, and likewise for the others. Strings are ordered by the values of their bytes.

The comparison operators do not associate: `a < b < c` is an error.

### A.5.6 Logical Operators

`not`, `and` and `or` take operands of type `bool` and yield a `bool`. The right operand of `and` is evaluated only if the left is `true`, and the right operand of `or` only if the left is `false`.

### A.5.7 Conditional Expressions

    if condition { expression } else { expression }

The two expressions have the same type, which is the type of the whole. The `else` may be followed by another `if` in place of a block.

    match expression {
        pattern => expression
        ...
    }

The expressions of the arms have the same type, which is the type of the whole. An arm whose body leaves by `return`, `fail`, `break`, `continue` or a call of `panic` has no type, and may stand among arms of any type. Patterns are described in A.8.

    x else y

If `x` is an optional `T?` or a result `T ! E`, and `y` is a `T`, the value is that of `x` if it has one, and otherwise `y`, which is not evaluated unless it is needed. `y` may also be an optional or a result of `T`, and then so is the whole. In place of `y` there may be a block that leaves, or a `fail`. For a result,

    x else name => y

binds the error to `name` in `y`.

### A.5.8 Lambdas

    name -> body
    (name, name, ...) -> body
    (name: Type, ...) -> body
    () -> body

The body is an expression, whose value the lambda returns, or a block. The types of the parameters are those that the context requires; where the context does not determine them they are written.

A lambda may use the variables of the enclosing function. If the lambda is written as an argument to a parameter of a function type that is not `take`, it uses those variables directly, and may assign to them. Otherwise the lambda holds a copy of the value of each variable it uses, made when the lambda expression is evaluated, and may not assign to them.

A function that receives a function as a parameter that is not `take` may only call it or pass it to another such parameter.

### A.5.9 Interpolation

Inside a string constant or a text block, `{` *expression* `}` is replaced by the text form of the value of the expression (A.9.1), and `{` *expression* `:` *format* `}` by that text laid out according to the format, which consists of, in order and each optionally:

- `<`, `>` or `^`: the text is placed at the left, at the right, or in the middle of the field;
- `0`: a number is padded with zeros after its sign instead of with blanks;
- a decimal number, the least width of the field, in characters;
- `.` and a decimal number, for a `float` the number of digits after the decimal point.

Without an alignment, a number is placed at the right and any other value at the left. A text longer than the width is not shortened.

## A.6 Statements

A function body, and every block, is a sequence of statements, one to a line. A block introduces a scope. A name is in scope from its declaration to the end of the innermost block that contains the declaration. A name may not be declared twice in one block; it may be declared in a block nested inside the scope of another declaration of the same name, which it then hides.

### A.6.1 Declarations

    name := expression
    name: Type = expression
    var name := expression
    var name: Type = expression
    pattern := expression

The first two declare a name that cannot be assigned to; the next two, a variable. The last declares the names in a tuple pattern (A.8), from the elements of a tuple; it may be preceded by `var`. Every declaration has an initial value.

### A.6.2 Assignments

    place = expression
    place += expression        and likewise -=, *=, /=

A *place* is a variable declared with `var`, or a `var` parameter or receiver, optionally followed by any number of field selections `.f` and list or map subscripts `[i]`. The compound forms are not allowed when the place ends in a map subscript. `+=` on a string appends. For a `Duration` or a `Time` the compound forms are those whose operator yields the type of the place. An assignment has no value, and one assignment assigns to one place.

### A.6.3 Expression Statements

An expression may be a statement if it is a call, or ends in `?`, or is an `else` whose alternative leaves. If the value of a call used as a statement is a result, it is an error not to handle it.

### A.6.4 Conditional Statements

    if condition { ... }
    if condition { ... } else { ... }
    if condition { ... } else if condition { ... } else { ... }

A condition is an expression of type `bool`, or a *binding condition* `pattern := expression`, where the expression is an optional. A binding condition holds when the optional has a value, and declares the names of the pattern, from that value, in the block that follows.

    match expression {
        pattern => statement-or-block
        pattern if condition => statement-or-block
        ...
    }

The arms are tried in order, and the first whose pattern matches the value of the expression, and whose condition, if it has one, is true, is executed. The patterns must be exhaustive: it is an error if the compiler cannot show that some arm matches every possible value. The body of an arm is a block, or a single assignment, call, `return`, `fail`, `break` or `continue`.

### A.6.5 Loops

    while condition { ... }

The condition, which may be a binding condition, is evaluated before each execution of the block, and the loop ends when it does not hold.

    for pattern in sequence { ... }

The sequence is a range, a list, a map or a set. It is evaluated once. The block is executed once for each integer of the range, element of the list, entry of the map as a tuple `(key, value)`, or member of the set, in order; the order of a map or set is that in which the keys or members were first added. The names of the pattern cannot be assigned to.

`break` ends the innermost enclosing loop. `continue` begins its next iteration.

### A.6.6 Leaving a Function

    return
    return expression
    fail expression

`return` ends the function, with the value of the expression as its result if the function has a return type. `fail` ends the function with an error, which must be of one of the types of the function's error set. The error may be a variable that an error arm of a `match`, or an `else`, bound to an error of any of several types; then every one of those types must be in the set. A function that has a return type must end every path through its body with a `return`, a `fail`, or a call of `panic`.

## A.7 Declarations of Functions and Types

### A.7.1 Functions

    fn name(parameters) -> Type ! Errors {
        ...
    }

`-> Type` is omitted for a function that returns nothing, and `! Errors` for one that cannot fail. `Errors` is one or more error types or named error sets, separated by `|`, or `Failure`, which stands for every error.

A parameter is `name: Type`, `name: var Type` or `name: take Type`, optionally followed, in the first form, by `= constant`, a default value. The function may not assign to a parameter of the first or the third kind. A `var` parameter denotes the place that was passed, and assignments to it change that place.

A *method* has a receiver, named `self`, before its name:

    fn (self: Type) name(parameters) ...
    fn (self: var Type) name(parameters) ...

and is called as `value.name(arguments)`. A method with a `var` receiver may be called only on a place. A *function of a type*,

    fn Type.name(parameters) ...

is called as `Type.name(arguments)`.

A function or method may have *type parameters*, written in angle brackets after its name, each optionally with a constraint: `fn largest<T: Ordered>(items: [T]) -> T?`. A constraint is an interface (A.7.5) or one of `Equatable`, `Ordered` and `Hashable`. A type parameter written after a `!`, in the type of a function parameter and in the function's own error set, as in `fn retry<T, E>(body: fn() -> T ! E) -> T ! E`, stands for the errors of the function that is passed, which may be none. A parameter whose type is written as an interface is an abbreviation for a parameter of a new type parameter with that constraint. The type arguments of a call are inferred from its arguments.

The function `main`, with no parameters and no return type, is where execution begins. It may have an error set.

### A.7.2 Structures and Aliases

    type Name = { field: Type, field: Type = constant, ... }

declares a structure type. A value is written `Name { field: expression, ... }`; a field with a default may be omitted, and `field` alone abbreviates `field: field`. A structure type may have type parameters, `type Stack<T> = { items: [T] }`, and where the type arguments of a literal cannot be inferred they are written, `Stack<int> { items: [] }`.

A structure may not contain a field of its own type, directly or as an optional. It may contain a list of its own type.

    type Name = Type

where `Type` is not a list of fields in braces, declares `Name` as another name for an existing type.

### A.7.3 Enumerations

    enum Name {
        Variant
        Variant { field: Type, ... }
        ...
    }

declares a type whose values are its variants. The variants may be separated by commas in place of line ends. A variant is written `Name.Variant`, or `Name.Variant { field: expression, ... }` if it has fields. An enumeration may have type parameters.

### A.7.4 Errors

    error Name
    error Name { field: Type, ... }
    error Name = Error1 | Error2 | ...

The first two declare an error type, whose values are written `Name` and `Name { field: expression, ... }`. The third names a set of error types. The predeclared error types are `ParseError { input: string }`, `Overflow`, `IoError { message: string }` and `JsonError { message: string, path: string }`.

### A.7.5 Interfaces

    interface Name {
        fn method(parameters) -> Type
        ...
    }

A type satisfies the interface if it has a method of each name with the parameters and result given. An interface may be used as a constraint, and as the type of a parameter (A.7.1). It may not be used as the type of a variable, a field or an element.

### A.7.6 Actors

    actor Name {
        field: Type
        var field: Type = constant
        ...
        fn method(parameters) -> Type ! Errors {
            ...
        }
        ...
    }

declares an actor, and the type `Name` of its handles. In the body of a method, the fields are in scope as names; a field declared with `var` may be assigned to, and the other methods may be called by their names.

`Name.start(field: expression, ...)` creates an actor, with a value for each field that has no default, and returns a handle. A handle is a value like another: it is copied, compared with `==`, kept in a list or a structure, and sent in a message; as text it is the actor's type and a number, `Counter#3`, unless the actor has a method `to_string`. For a handle `h`:

- `h.method(arguments)` *asks*: it executes the method with exclusive use of the actor's fields and yields its result, waiting first for any other execution of a method of that actor, and for every message told to the actor before, to end;
- `h.send.method(arguments)` *tells*, and is allowed for a method with no return type and no error set: the method will be executed later, on another thread, after the messages told to that actor earlier. If as many messages are waiting for the actor as its mailbox holds, the thread waits until there are fewer;
- `h.try_send.method(arguments)` tells if the mailbox has room, and yields a `bool` that says whether it did;
- `h.stop()` stops the actor.

A mailbox holds 1,024 messages, or the number given by `@mailbox(n)` on a line before the declaration of the actor.

The types of a method's parameters and result, and of the fields given to `start`, are restricted to those that can be messages: `int`, `float`, `bool`, `string`, `Duration`, `Time`, lists and optionals of message types, maps from `string` to a message type, structures, enumerations and errors whose fields are message types, and handles. The fields given to `start` may in addition be of types that the others exclude, provided they contain no function.

A program ends when `main` has returned and every message that was told has been executed.

## A.8 Patterns

A pattern is matched against a value, and may declare names.

| Pattern | Matches |
| --- | --- |
| `_` | any value |
| *name* | any value, which the name is declared as |
| *constant* | an `int`, `string` or `bool` equal to the constant; a floating constant is not a pattern |
| `a..b`, `a..=b` | an `int` in the range, where `a` and `b` are integer constants |
| `none` | an optional without a value |
| `(p1, p2, ...)` | a tuple whose elements match the patterns |
| `Name { f: p, g, .. }` | a structure of type `Name`, or a value of the variant `Name`, whose field `f` matches `p`; `g` alone abbreviates `g: g`; `..` stands for the fields not named, and without it every field is named |
| `Name` | a value of the variant `Name`, whatever its fields |
| `p1 \| p2` | a value that matches either pattern |
| `! p` | in a `match` on a result: an error that matches `p`, where `p` is an error type, an error type with fields in braces, `name: Type`, or a name |

In a `match` on an optional, a pattern other than `none` is matched against the value that the optional holds; in a `match` on a result, a pattern without `!` is matched against the value of a success.

In a declaration, a `for` statement and a binding condition, a pattern is a name, `_`, or a tuple of these.

## A.9 Built-in Functions

`print(value)` writes the text form of a value and a newline on the standard output. `panic(message)` writes a message on the standard error output and ends the program with status 101.

The functions of the basic types, lists, strings, maps, sets and optionals are listed in Appendix B.

### A.9.1 Text Form

The text form of a value is:

- for an `int` or `byte`, its decimal digits, with a minus sign if it is negative;
- for a `float`, its decimal digits, always with a decimal point or an exponent, so that it cannot be taken for an integer; or `inf`, `-inf` or `NaN`;
- for a `bool`, `true` or `false`;
- for a `Duration`, a duration constant, with a minus sign if it is negative, and `0s` for none; for a `Time`, the date and time in UTC as RFC 3339 writes them, `2026-01-31T12:00:00Z`, with a fraction of a second if there is one;
- for a string, its characters; but where the string is an element or field of another value, the characters in double quotes, with escape sequences;
- for a list, `[e1, e2]`; for a map, `[k1: v1, k2: v2]`; for a set, `Set[e1, e2]`; for a tuple, `(e1, e2)`;
- for an optional, the text form of its value, or `none`;
- for a structure, `Name { field: value, ... }`, with type arguments after the name if it has them; for a variant or an error, the same with the name of the variant or error, and the name alone if it has no fields;
- for a type that has a method `to_string() -> string`, the result of that method, wherever a value of the type appears.

## A.10 Panics

A program *panics* when an `int` operation overflows; when an integer is divided by zero; when a list is subscripted out of range, or `remove`, `insert` or `swap` is given a position that does not exist; when a string is sliced at a position that is out of range or inside a character; when an actor that is stopped is asked; when an ask would wait for an actor that is waiting for the asker, or a tell for room in the mailbox of one; when a thread's stack is exhausted, by calls nested too deeply or by the release of a value nested too deeply; and when `panic` is called.

A panic writes `panic:`, a message, and for most panics the file and line, on the standard error output. Unless it occurs in a message that was told to an actor, it ends the program with status 101; an exhausted stack ends the program wherever it occurs. In a message that was told, it ends the message; each actor that was in the middle of a method at that point is started again with the fields it was started with, unless it has been started again five times within the last minute, in which case it is stopped.
