# Chapter 6: Structures

A structure is a collection of one or more values, possibly of different types, grouped together under a single name for convenient handling. Structures help to organize complicated data, particularly in large programs, because they permit a group of related values to be treated as a unit instead of as separate entities.

One traditional example of a structure is the payroll record: an employee is described by a set of attributes such as name, address, salary, and so on. Another example, more typical for Koo, comes from the programs it is meant for: an order is an identifier, a customer, and a list of the things ordered, each of which is itself a structure.

This chapter also covers the other ways of making a type of your own: enumerations, whose values are one of several alternatives; type parameters, which let one definition serve many types; and interfaces, which say what a type parameter must be able to do.

## 6.1 Basics of Structures

Let us create a few structures suitable for graphics. The basic object is a point, which we will assume has an *x* coordinate and a *y* coordinate, both integers. The two components can be placed in a structure declared like this:

    type Point = { x: int, y: int }

The word `type` introduces a type declaration. After it come the name of the new type, which begins with a capital letter, an `=`, and the list of its *fields* in braces, each a name and a type. The fields are separated by commas, or by the ends of lines when they are written one per line.

A value of the type is written as the name of the type and, in braces, a value for each field:

    corner := Point { x: 320, y: 200 }

A field of a structure is referred to by a construction of the form *structure*`.`*field*. To print the coordinates of the point `corner`, for instance,

    print("{corner.x}, {corner.y}")

Structures can be nested. One representation of a rectangle is a pair of points that denote the diagonally opposite corners:

```koo
type Point = { x: int, y: int }
type Rect = { min: Point, max: Point }

fn main() {
    corner := Point { x: 320, y: 200 }
    print("{corner.x}, {corner.y}")

    screen := Rect { min: Point { x: 0, y: 0 }, max: corner }
    print(screen.max.x)
    print(screen)
    print(screen.min == Point { x: 0, y: 0 })

    var moved := corner
    moved.x += 10
    print("{moved} {corner}")
}
```

```text
320, 200
320
Rect { min: Point { x: 0, y: 0 }, max: Point { x: 320, y: 200 } }
true
Point { x: 330, y: 200 } Point { x: 320, y: 200 }
```

The program shows several things that every structure can do without your writing a line for it. It can be printed, in a form that shows its fields. It can be compared with `==`, which is true when all the fields are equal. And it is a value: `moved := corner` makes an independent point, and changing one leaves the other alone. A field can be assigned only if the structure is in a variable declared with `var`.

When the value for a field is a variable of the same name, the name need be written only once, and a field may be given a *default*, which is the value it has when the literal does not mention it:

```koo
type Server = { host: string = "localhost", port: int = 8080, name: string }

fn main() {
    name := "orders"
    print(Server { name })
    print(Server { name, port: 9000 })
}
```

```text
Server { host: "localhost", port: 8080, name: "orders" }
Server { host: "localhost", port: 9000, name: "orders" }
```

A field with no default must be given a value in every literal.

## 6.2 Structures and Functions

Structures are passed to functions and returned from them like any other values. Let us investigate by writing some functions to manipulate points and rectangles. The first, `add_point`, adds two points; the second tests whether a point is inside a rectangle, where we have adopted the convention that a rectangle includes its left and bottom sides but not its top and right sides:

```koo
type Point = { x: int, y: int }
type Rect = { min: Point, max: Point }

// Add two points.
fn add_point(a: Point, b: Point) -> Point {
    return Point { x: a.x + b.x, y: a.y + b.y }
}

// Whether p is in r.
fn in_rect(p: Point, r: Rect) -> bool {
    return p.x >= r.min.x and p.x < r.max.x and p.y >= r.min.y and p.y < r.max.y
}

// The rectangle with the same corners, in canonical form.
fn canon_rect(r: Rect) -> Rect {
    return Rect {
        min: Point { x: r.min.x.min(r.max.x), y: r.min.y.min(r.max.y) },
        max: Point { x: r.min.x.max(r.max.x), y: r.min.y.max(r.max.y) },
    }
}

fn main() {
    origin := Point { x: 0, y: 0 }
    screen := Rect { min: origin, max: Point { x: 320, y: 200 } }
    middle := Point { x: (screen.min.x + screen.max.x) / 2, y: (screen.min.y + screen.max.y) / 2 }

    print(add_point(middle, Point { x: 10, y: 10 }))
    print(in_rect(middle, screen))
    print(in_rect(screen.max, screen))
    print(canon_rect(Rect { min: screen.max, max: origin }))
}
```

```text
Point { x: 170, y: 110 }
true
false
Rect { min: Point { x: 0, y: 0 }, max: Point { x: 320, y: 200 } }
```

In C, a large structure is generally passed by a pointer to it, for efficiency, and the function must then be trusted not to change it. In Koo there is nothing to decide. A structure passed to a plain parameter is lent, at no cost whatever its size, and cannot be changed by the function.

## 6.3 Methods

The functions of the last section belong with the types they work on, and Koo lets you say so. A *method* is a function whose first parameter, the *receiver*, is written before the function's name, and is always called `self`:

    fn (self: Point) add(other: Point) -> Point {

A method is called with the receiver in front: `p.add(q)`. Apart from where the receiver is written, a method is an ordinary function. In particular the receiver is one of the three kinds of parameter. A method that changes its receiver says `self: var Point`, and can be called only on a variable.

A function may also be attached to the type itself, with no receiver. This is the customary way to provide the functions that make values of a type:

    fn Point.origin() -> Point {

and it is called as `Point.origin()`.

```koo
type Point = { x: int, y: int }

fn Point.origin() -> Point {
    return Point { x: 0, y: 0 }
}

fn (self: Point) add(other: Point) -> Point {
    return Point { x: self.x + other.x, y: self.y + other.y }
}

fn (self: Point) length_squared() -> int {
    return self.x * self.x + self.y * self.y
}

fn (self: var Point) move_by(dx: int, dy: int) {
    self.x += dx
    self.y += dy
}

fn main() {
    var p := Point.origin().add(Point { x: 3, y: 4 })
    print(p.length_squared())
    p.move_by(1, 1)
    print(p)
}
```

```text
25
Point { x: 4, y: 5 }
```

Unlike a `var` argument, a `var` receiver is not marked at the call. It does not need to be: in `p.move_by(1, 1)` the thing that changes is already the first thing on the line.

Two methods have a meaning to the compiler. If a type has a method `to_string()` that returns a string, then that is how its values are printed, in place of the form that shows the fields. And if a type has a method `compare(other)` that returns an integer, negative, zero or positive according as the receiver comes before, with or after the other, then its values can be compared with `<`, `<=`, `>` and `>=`, and lists of them can be sorted.

```koo
type Version = { major: int, minor: int }

fn (self: Version) to_string() -> string {
    return "v{self.major}.{self.minor}"
}

fn (self: Version) compare(other: Version) -> int {
    if self.major != other.major {
        return self.major.compare(other.major)
    }
    return self.minor.compare(other.minor)
}

fn main() {
    var versions := [
        Version { major: 2, minor: 0 },
        Version { major: 1, minor: 12 },
        Version { major: 1, minor: 5 },
    ]
    versions.sort()
    print(versions)
    print(versions[0] < versions[1])
    print("the newest is {versions.max()}")
}
```

```text
[v1.5, v1.12, v2.0]
true
the newest is v2.0
```

The basic types have `compare` themselves, as the method does for its fields.

## 6.4 Lists of Structures

Consider writing a program to count the occurrences of each Koo keyword. We need a list of strings to hold the names, and a list of integers for the counts. One possibility is to use two parallel lists. But the very fact that the lists are parallel suggests a different organization, a list of structures. Each keyword entry is a pair:

    type Key = { word: string, count: int }

and there is a list of pairs. Since the list is in alphabetical order, the entry for a word can be found by binary search.

```koo
import std/io

type Key = { word: string, count: int }

// Count some of the keywords of Koo.
fn main() {
    var keys := ["break", "continue", "else", "fn", "for", "if", "return", "var", "while"]
        .map(word -> Key { word, count: 0 })
    while line := io.read_line() {
        for word in words_of(line) {
            if i := bin_search(word, keys) {
                keys[i].count += 1
            }
        }
    }
    for key in keys {
        if key.count > 0 {
            print("{key.count:4} {key.word}")
        }
    }
}

// Find the word in keys[0]...keys[keys.len - 1].
fn bin_search(word: string, keys: [Key]) -> int? {
    var low := 0
    var high := keys.len - 1
    while low <= high {
        mid := (low + high) / 2
        if word < keys[mid].word {
            high = mid - 1
        } else if word > keys[mid].word {
            low = mid + 1
        } else {
            return mid
        }
    }
    return none
}

// The words of a line: its runs of letters and underscores.
fn words_of(line: string) -> [string] {
    var words: [string] = []
    var word := ""
    for c in line.chars() {
        if (c >= "a" and c <= "z") or (c >= "A" and c <= "Z") or c == "_" {
            word += c
        } else if word != "" {
            words.push(word)
            word = ""
        }
    }
    if word != "" {
        words.push(word)
    }
    return words
}
```

```input
fn main() {
    var n := 0
    for i in 0..10 {
        if i % 2 == 0 { n += i } else { continue }
    }
    while n > 0 { n -= 1 }
}
```

```text
   1 continue
   1 else
   1 fn
   1 for
   1 if
   1 var
   1 while
```

`keys[i].count += 1` changes one field of one element of the list, in place. A field of an element of a list of a field is as much a variable as a plain name is, provided the name at the bottom of it all was declared with `var`.

A map from words to counts would do this job in fewer lines, as Section 5.8 showed. The list of structures has the merit of holding exactly the words that are wanted, in an order that was chosen.

**Exercise 6-1.** Our version of `words_of` does not properly handle string constants or comments. Write a better version.

## 6.5 Self-referential Structures

Suppose we want to handle the more general problem of counting the occurrences of *all* the words in some input. Since the list of words is not known in advance, we cannot conveniently sort it and use a binary search. Yet we cannot do a linear search for each word as it arrives, to see if it has already been seen; the program would take too long. How can we organize the data to cope efficiently with a list of arbitrary words?

One solution is to keep the set of words seen so far sorted at all times, by placing each word into its proper position in the order as it arrives. This should not be done by shifting words in a linear list, though; that also takes too long. Instead we will use a data structure called a *binary tree*.

The tree contains one node per distinct word; each node contains

- the text of the word,
- a count of the number of occurrences,
- the subtree of the words that come before it,
- the subtree of the words that come after it.

No node may have more than two children; it might have only zero or one.

The nodes are maintained so that at any node the left subtree contains only words that are lexicographically less than the word at the node, and the right subtree contains only words that are greater. To find out whether a new word is already in the tree, start at the root and compare the new word to the word stored at that node. If they match, the question is answered affirmatively. If the new word is less than the tree word, continue searching at the left child, otherwise at the right child. If there is no child in the required direction, the new word is not in the tree, and in fact the empty slot is the proper place to add the new word. This process is recursive, since the search from any node uses a search from one of its children.

A structure cannot contain a value of its own type directly: a node that contained two nodes would contain four, and so on without end. It can, however, contain a *list* of its own type, and a list may be empty. So we represent a subtree as a list of nodes that holds either one node, the root of the subtree, or none:

    type Node = { word: string, count: int, left: [Node], right: [Node] }

(The more natural declaration would make the subtrees optional nodes, `left: Node?`. The compiler does not accept a structure that contains itself in that way yet, and says so.)

The code for the whole program is surprisingly small. The function `add` is recursive. A word is presented by `main` to the top level (the root) of the tree. At each stage, that word is compared to the word already stored at the node, and is percolated down to either the left or right subtree by a recursive call to `add`. Eventually the word either matches something already in the tree (in which case the count is incremented), or an empty subtree is encountered, indicating that a node must be created and added to the tree.

```koo
import std/io

// A node of the tree: a word, how often it was seen, and the subtrees of the words
// before it and after it. A subtree is a list of one node, or of none.
type Node = { word: string, count: int, left: [Node], right: [Node] }

// Word frequency count.
fn main() {
    var root: [Node] = []
    while line := io.read_line() {
        for word in line.split(" ") {
            if word != "" {
                add(var root, word)
            }
        }
    }
    show(root)
}

// Add a word at or below the root of a subtree.
fn add(tree: var [Node], word: string) {
    if tree.is_empty() {
        tree.push(Node { word, count: 1, left: [], right: [] })     // a new word has arrived
    } else if word == tree[0].word {
        tree[0].count += 1                                           // a repeated word
    } else if word < tree[0].word {
        add(var tree[0].left, word)                                  // into the left subtree
    } else {
        add(var tree[0].right, word)                                 // into the right subtree
    }
}

// Print a subtree in order.
fn show(tree: [Node]) {
    for node in tree {
        show(node.left)
        print("{node.count:4} {node.word}")
        show(node.right)
    }
}
```

```input
now is the time for all good men
to come to the aid of their party
```

```text
   1 aid
   1 all
   1 come
   1 for
   1 good
   1 is
   1 men
   1 now
   1 of
   1 party
   2 the
   1 their
   1 time
   2 to
```

`add` is lent the subtree it is to change, with `var`, and lends in turn the left or right subtree of its root to the next call. No part of the tree is copied on the way down. The whole tree belongs to the variable `root` in `main`, and when `main` ends, every node is freed.

`show` prints the tree in sorted order; at each node, it prints the left subtree (all the words less than this word), then the word itself, then the right subtree (all the words greater). If you feel shaky about how recursion works, simulate `show` as it operates on the tree for the input above.

A practical note: if the tree becomes "unbalanced" because the words don't arrive in random order, the running time of the program can grow too much. As a worst case, if the words are already in order, this program does an expensive simulation of linear search.

**Exercise 6-2.** Write a program that reads its input and prints the distinct words in order of decreasing frequency of occurrence. Precede each word by its count.

## 6.6 Enumerations

A structure has all of its fields at once. An *enumeration* is a type whose value is exactly one of a fixed set of *variants*. The simplest enumerations are lists of names:

    enum Color { Red, Green, Blue }

A variant is referred to as `Color.Red`. Like every value, a variant can be printed and compared with `==`.

A variant may also carry fields, and different variants may carry different ones. Such an enumeration does the job of a `union` in C, with the important difference that the value itself knows which variant it is, so that it is impossible to store one kind of thing and take out another.

```koo
enum Shape {
    Circle { radius: float }
    Rect { width: float, height: float }
    Dot
}

fn area(shape: Shape) -> float {
    return match shape {
        Circle { radius } => 3.14159 * radius * radius
        Rect { width, height } => width * height
        Dot => 0.0
    }
}

fn main() {
    shapes := [
        Shape.Circle { radius: 1.0 },
        Shape.Rect { width: 2.0, height: 3.0 },
        Shape.Dot,
    ]
    for shape in shapes {
        print("{shape} has area {area(shape):.2}")
    }
}
```

```text
Circle { radius: 1.0 } has area 3.14
Rect { width: 2.0, height: 3.0 } has area 6.00
Dot has area 0.00
```

The variants are written one to a line, or on one line with commas between. A value of a variant with fields is written like a structure, with the name of the variant in place of the name of a type.

The way to use an enumeration is `match`. A pattern in an arm is the name of a variant, without the name of the enumeration, since the type of the value being matched is known; after it, in braces, the fields that are wanted are named, and each becomes a variable in the arm. A pattern may ask for a particular value of a field, in the manner of `Point { x: 0, y }` in the next example, and may end with `..` to pass over the fields that it does not name.

Here the requirement that a `match` be exhaustive shows its worth. Let us add a variant to `Shape`, and forget to say what its area is:

```koo
enum Shape {
    Circle { radius: float }
    Rect { width: float, height: float }
    Triangle { base: float, height: float }
    Dot
}

fn area(shape: Shape) -> float {
    return match shape {
        Circle { radius } => 3.14159 * radius * radius
        Rect { width, height } => width * height
        Dot => 0.0
    }
}

fn main() {
    print(area(Shape.Dot))
}
```

```error
error: this `match` does not cover `Triangle`
 --> main.koo:9:18
  |
9 |     return match shape {
  |                  ^^^^^ this is a `Shape`
  |
  = note: a `match` covers every case (Section 6.6)
  = help: add an arm for each, or a last arm `_ => ...` for every case not named

1 error
```

In a large program the definition of `Shape` and the functions that match on it may be far apart, and written by different people. The compiler finds every one of them.

Enumerations have methods just as structures do, and structures can be matched just as enumerations are:

```koo
enum Light { Red, Amber, Green }

fn (self: Light) next() -> Light {
    return match self {
        Red => Light.Green
        Green => Light.Amber
        Amber => Light.Red
    }
}

type Point = { x: int, y: int }

fn where_is(p: Point) -> string {
    return match p {
        Point { x: 0, y: 0 } => "at the origin"
        Point { x: 0, y } => "on the y axis at {y}"
        Point { x, y: 0 } => "on the x axis at {x}"
        Point { .. } => "somewhere else"
    }
}

fn main() {
    var light := Light.Red
    for _ in 0..4 {
        light = light.next()
        print(light)
    }
    print(where_is(Point { x: 0, y: 7 }))
    print(where_is(Point { x: 3, y: 4 }))
}
```

```text
Green
Amber
Red
Green
on the y axis at 7
somewhere else
```

The types of a package that a program imports (Section 4.4) are offered as its functions are: a structure, an enumeration, an error or an interface that is declared `pub` is written by those who import the package with the package's name before its own, `shapes.Circle`, and so is a variant, `shapes.Kind.Round`. Two things about a `pub` type remain the package's own. The first is whatever of it is not offered: a method is declared `pub` like any function, and a field whose name begins with an underscore is one that only the package reads and sets, so that the package may keep a value consistent that others are free to hold and pass around. The second is the future of an enumeration. A package may add a variant later, and a program that matched every variant there was would then stop compiling, for a change that broke nothing else; so a `match` on another package's enumeration ends with an arm `_`, unless the package has promised with `@closed` that the variants are final.

```koo
import ./shapes

fn main() {
    circle := shapes.Circle.of(2.0)
    print("{circle.radius} {circle.area():.2} {circle}")
    for kind in [shapes.Kind.Round, shapes.Kind.Square] {
        match kind {
            shapes.Kind.Round => print("round")
            _ => print("not round")
        }
    }
    match shapes.Side.Left {
        shapes.Side.Left => print("left")
        shapes.Side.Right => print("right")
    }
}
```

```file shapes/shapes.koo
pub enum Kind { Round, Square }

@closed
pub enum Side { Left, Right }

pub type Circle = { radius: float, _area: float = 0.0 }

pub fn Circle.of(radius: float) -> Circle {
    return Circle { radius, _area: 3.14159 * radius * radius }
}

pub fn (self: Circle) area() -> float {
    return self._area
}
```

```text
2.0 12.57 shapes.Circle { radius: 2.0 }
round
not round
left
```

**Exercise 6-3.** The calculator of Section 4.3 examines each word of its input twice: once to see whether it is an operator, and again when it operates. Define an enumeration `Token` with a variant for a number and one for an operator, write a function that turns a word into a `Token?`, and rewrite the calculator to use it.

## 6.7 Names for Types

A type declaration can also give a new name to a type that exists:

```koo
type Celsius = float
type Table = [string: [int]]

fn warmer(t: Celsius) -> Celsius {
    return t + 1.5
}

fn main() {
    var seen: Table = [:]
    seen["primes"] = [2, 3, 5]
    print(seen)
    print(warmer(20.0))
}
```

```text
["primes": [2, 3, 5]]
21.5
```

Such a name is an *alias*, like a `typedef` in C. It does not make a new type: a `Celsius` is a `float` in every respect, and may be used wherever a `float` is. An alias is worth having when it makes a long type shorter, or when it says what a value is for.

## 6.8 Type Parameters

A stack of integers and a stack of strings differ in nothing but the type of their elements. To write the code once, a type or a function is given a *type parameter*, a name in angle brackets that stands for a type to be supplied later.

```koo
type Stack<T> = { items: [T] }

fn Stack<T>.new() -> Stack<T> {
    return Stack { items: [] }
}

fn (self: var Stack<T>) push(item: T) {
    self.items.push(item)
}

fn (self: var Stack<T>) pop() -> T? {
    return self.items.pop()
}

fn (self: Stack<T>) depth() -> int {
    return self.items.len
}

fn main() {
    var words := Stack<string>.new()
    words.push("first")
    words.push("second")
    print(words.pop())
    print(words.depth())

    var numbers := Stack<float>.new()
    numbers.push(2.5)
    print(numbers)
}
```

```text
second
1
Stack<float> { items: [2.5] }
```

`Stack<string>` and `Stack<float>` are different types, made from the one definition. The compiler generates the code of `Stack` separately for each type it is used with, so a `Stack<float>` is as fast as if it had been written by hand for `float`, and a `Stack<string>` cannot have a number pushed on it by mistake. Lists, maps, sets and optionals are types with parameters too, with a notation of their own.

A function may have type parameters as well. They are written after its name, and are usually found from the arguments, so that the call looks like any other:

```koo
// The first element of a list, or a value to use if it is empty.
fn first_or<T>(items: [T], fallback: T) -> T {
    return items.first() else fallback
}

// The largest element of a list.
fn largest<T: Ordered>(items: [T]) -> T? {
    if items.len == 0 {
        return none
    }
    var best := items[0]
    for item in items {
        if item > best {
            best = item
        }
    }
    return best
}

fn main() {
    print(first_or([4, 5], 0))
    print(first_or(["x"].skip(1), "nothing"))
    print(largest([3, 9, 4]))
    print(largest(["pear", "apple", "quince"]))
    print(largest([2.5, 1.5]))
}
```

```text
4
nothing
9
quince
2.5
```

`first_or` works for a list of anything, because it does nothing with the elements but hand them on. `largest` compares its elements with `>`, and not every type has an order. The *constraint* after the colon, `T: Ordered`, says that `largest` may be used only with types that do; in return, the body may use the relational operators on values of type `T`. Three constraints are built in:

| | |
| --- | --- |
| `Equatable` | the type has `==` and `!=` |
| `Ordered` | it has `<`, `<=`, `>`, `>=` and `compare`: numbers, strings, and any type you give a `compare` method |
| `Hashable` | it can be the key of a map or a member of a set |

A constraint is checked at each call. A list of lists has no order, so `largest` cannot be given one, and the message is about the call:

```koo
fn largest<T: Ordered>(items: [T]) -> T? {
    if items.len == 0 {
        return none
    }
    var best := items[0]
    for item in items {
        if item > best {
            best = item
        }
    }
    return best
}

fn main() {
    print(largest([[1, 2], [3]]))
}
```

```error
error: `[int]` does not satisfy `Ordered`
  --> main.koo:15:11
   |
15 |     print(largest([[1, 2], [3]]))
   |           ^^^^^^^^^^^^^^^^^^^^^^
   |
   = help: `largest` requires it: it has no order; give the type a method `fn (self: T) compare(other: T) -> int`

1 error
```

You should know that the compiler does not yet hold a function to its own constraints. It checks the body of a function with type parameters separately for each type that the function is used with, and not once, in general, against what the first line promises. If `largest` had been written without `: Ordered`, it would still be accepted, and would still work for numbers and strings; the call with a list of lists would then be reported from inside the function, at the `>`, which is the wrong place to learn of it. Write the constraint. It tells a caller what the function needs, and it puts the message where the mistake is.

## 6.9 Interfaces

The built-in constraints are instances of a general mechanism. An *interface* is a set of methods, and a type *satisfies* the interface if it has them. An interface is used as a constraint, to say what a function needs of a type.

```koo
interface Shape {
    fn area() -> float
    fn name() -> string
}

type Square = { side: float }

fn (self: Square) area() -> float {
    return self.side * self.side
}

fn (self: Square) name() -> string {
    return "square"
}

type Circle = { radius: float }

fn (self: Circle) area() -> float {
    return 3.14159 * self.radius * self.radius
}

fn (self: Circle) name() -> string {
    return "circle"
}

fn describe(shape: Shape) -> string {
    return "a {shape.name()} of area {shape.area():.1}"
}

fn total_area<T: Shape>(shapes: [T]) -> float {
    return shapes.map(s -> s.area()).sum()
}

fn main() {
    print(describe(Square { side: 2.0 }))
    print(describe(Circle { radius: 1.0 }))
    print(total_area([Square { side: 1.0 }, Square { side: 2.0 }]))
}
```

```text
a square of area 4.0
a circle of area 3.1
5.0
```

Nothing in the declarations of `Square` and `Circle` mentions `Shape`. A type satisfies an interface by having the methods, and need not know that the interface exists; so an interface can be written afterwards, for types that are already there, including types that someone else wrote.

An interface may be written as the type of a parameter, as in `describe`. That is shorthand for a type parameter with the interface as its constraint: `describe` is compiled once for `Square` and once for `Circle`, and each version calls the right `area` directly.

What an interface cannot be, at present, is the type of a value. There is no list whose elements are "shapes of various types"; the list given to `total_area` is a list of squares. When values of several kinds must live in one list, the kinds are the variants of an enumeration, as in Section 6.6. The compiler says so when you try:

```koo
interface Named {
    fn name() -> string
}

fn main() {
    things: [Named] = []
    print(things.len)
}
```

```error
error: the interface `Named` cannot be used as a type here yet
 --> main.koo:6:14
  |
6 |     things: [Named] = []
  |              ^^^^^
  |
  = help: this compiler supports an interface as a constraint (`T: Name`) and as a parameter's type; interface values are not supported yet

1 error
```

**Exercise 6-4.** Write a generic type `Queue<T>` with methods to add an element at the back and remove one from the front, and a function that says whether a `Queue<T>` contains a given element. What constraint does the function need?
