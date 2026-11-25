# Chapter 5: Lists, Strings and Maps

In *The C Programming Language* this is the chapter on pointers and arrays, and it is the longest in the book, because in C the two are how everything larger than a number is handled. A pointer is the address of something; an array is a row of things at an address; a string is an array of characters, and a function that is to change anything is given a pointer to it. Much of the skill of a C programmer is the skill of keeping track of what points where, and of who is to free it.

Koo has no pointers. This chapter is about what it has in their place: lists, strings, maps, sets, tuples and optionals, all of which are values. It begins with what that means.

## 5.1 Values and Storage

A variable holds a value. That is all there is to the model, and the rest of this section is consequences.

*Assignment gives a value.* After `b := a`, the names `a` and `b` have equal values and nothing else to do with each other. If `a` is a variable and is later changed, `b` is as it was. It does not matter whether the value is an integer or a list of a million structures.

*Passing gives a value.* A function is given the values of its arguments. It cannot change the caller's variables, except one that is passed with `var` to a parameter declared `var`, and then the caller can see it at the call.

*A part of a value is not a thing on its own.* There is no way to get hold of "the third element of this list" and keep it, so that a later change to the list changes what you hold. You can copy the element; you can change it where it is, as `v[2] = 0` or `bump(var v[2])`; but you cannot have a second name for it.

```koo
type Account = { owner: string, balance: int }

fn main() {
    var accounts := [
        Account { owner: "ada", balance: 100 },
        Account { owner: "bob", balance: 50 },
    ]
    backup := accounts
    var first := accounts[0]

    first.balance = 0
    accounts[1].balance += 25

    print(first)
    print(accounts)
    print(backup)
}
```

```text
Account { owner: "ada", balance: 0 }
[Account { owner: "ada", balance: 100 }, Account { owner: "bob", balance: 75 }]
[Account { owner: "ada", balance: 100 }, Account { owner: "bob", balance: 50 }]
```

(`Account` is a structure, declared with `type`; Chapter 6 is about structures.) `first` is a copy of the first account, so emptying it does nothing to the list. `backup` is the list as it was, whatever is done to `accounts` afterwards.

You would be right to wonder what this costs. If every assignment of a list copied the list, programs would crawl. They do not, because of how values are stored. The elements of a list, and the bytes of a string, live in a piece of storage outside the variable, and the storage keeps a count of how many variables are using it. An assignment makes the new variable use the same storage, and adds one to the count: no elements are copied. Only when one of the holders is about to *change* the value does it matter that there are others, and then, if there are, it gets a copy of its own first. If it is the only holder, it changes the storage in place. The effect is always exactly as if every assignment had copied; the cost is that of copying only when two holders really do go separate ways.

Passing an argument to a plain parameter does not even touch the count. The function borrows the caller's storage for the length of the call.

Storage is freed when the count goes to zero: at the moment the last variable holding it goes out of scope or is given another value. There is no garbage collector to pause the program, and there is no `free` to forget or to call twice. Nor can a program reach storage that has been freed, since there is no way to hold on to storage except by holding a value.

## 5.2 Lists

A list is a sequence of values of one type. The type of a list whose elements are of type `T` is written `[T]`, so `[int]` is a list of integers and `[[string]]` a list of lists of strings. A list is written as its elements between brackets:

    primes := [2, 3, 5, 7]
    var names: [string] = []

The elements are numbered from zero. `v[i]` is element `i`, and `v.len` is the number of elements. Subscripts are checked: `v[i]` with `i` less than zero or not less than `v.len` is a panic.

```koo
fn main() {
    v := [10, 20, 30]
    i := 3
    print(v[i])
}
```

```panic
panic: index out of range
  at main.koo:4
```

When an index might be out of range and that is not a mistake, use `v.get(i)`, which is an optional: the element, or `none`. `v.first()` and `v.last()` are optionals too.

A list grows and shrinks at its end with `push` and `pop`, and anywhere with `insert` and `remove`:

```koo
fn main() {
    var v := [10, 20, 30]
    v.push(40)
    print(v)
    print(v.pop())
    v.insert(1, 15)
    print(v)
    print(v.remove(0))
    v.extend([50, 60])
    print(v)
    v.swap(0, 4)
    v.reverse()
    print(v)
    print("{v.len} elements, first {v.first()}, tenth {v.get(9)}")
    v.clear()
    print(v.is_empty())
}
```

```text
[10, 20, 30, 40]
40
[10, 15, 20, 30]
10
[15, 20, 30, 50, 60]
[15, 50, 30, 20, 60]
5 elements, first 15, tenth none
true
```

`pop` returns an optional, `none` when the list is empty; `remove` returns the element it removed, and panics if there is no such element. The methods that change a list can be called only on a list that is a `var`.

Some methods return a new list and leave the original alone:

```koo
fn main() {
    v := [3, 1, 4, 1, 5, 9, 2, 6]
    print(v.slice(2, 5))
    print(v.take(3))
    print(v.skip(5))
    print(v.reversed())
    print(v.sorted())
    print(v.contains(9))
    print(v.index_of(1))
    print(v.index_of(7))
    print("sum {v.sum()}, smallest {v.min()}, largest {v.max()}")
}
```

```text
[4, 1, 5]
[3, 1, 4]
[9, 2, 6]
[6, 2, 9, 5, 1, 4, 1, 3]
[1, 1, 2, 3, 4, 5, 6, 9]
true
1
none
sum 31, smallest 1, largest 9
```

`v.slice(from, to)` is the elements from position `from` up to but not including `to`. `take(n)` is the first `n` elements, or all of them if there are fewer, and `skip(n)` is the rest. `index_of` is an optional, because the element may not be there; so are `min` and `max`, because the list may be empty.

There are two ways to make a list without writing out its elements. `List<int>.with_size(n)` is a list of `n` zeros; the element type between the angle brackets may be `int`, `float`, `bool` or `byte`, and for `bool` the elements are `false`. `List<T>.with_capacity(n)` is an empty list of any type, with room set aside for `n` elements, which saves a little time when you know how long the list will get.

As an example of a list used the way C uses an array, here is the Sieve of Eratosthenes, which finds the prime numbers below a limit by crossing out the multiples of each prime in turn.

```koo
// Print the primes below 50.
fn main() {
    limit := 50
    var composite := List<bool>.with_size(limit)
    var primes: [int] = []
    for n in 2..limit {
        if composite[n] {
            continue
        }
        primes.push(n)
        var multiple := n * n
        while multiple < limit {
            composite[multiple] = true
            multiple += n
        }
    }
    print(primes)
}
```

```text
[2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47]
```

**Exercise 5-1.** Write a function `rotate(v, n)` that returns the list `v` with its elements moved `n` places to the left, those that fall off the front being put at the back.

## 5.3 Lists as Arguments

When a list is passed to a function, the function is lent the list. It is not given a pointer to the first element, as in C, with the length left behind: the list knows its length, and its subscripts are checked in the function as everywhere else.

To change the caller's list a function takes it as `var`. A single element of a list can be passed as `var` too, as can a field of a structure.

```koo
fn swap_ends(v: var [int]) {
    if v.len > 1 {
        v.swap(0, v.len - 1)
    }
}

fn double(n: var int) {
    n *= 2
}

fn main() {
    var v := [1, 2, 3, 4]
    swap_ends(var v)
    double(var v[1])
    print(v)
}
```

```text
[4, 4, 3, 1]
```

The alternative to changing a list in place is to return a new one, and it is often the better design, because a function that takes a value and returns a value can be used in the middle of an expression and cannot surprise its caller. The price is that the new list may have to be made by copying the old one.

## 5.4 Working on Whole Lists

Most loops over a list do one of a few things: make a new list from the old, element by element; pick out some of the elements; boil the elements down to a single value; look for one. Each of these is a method that takes a function, so that the loop does not have to be written out.

```koo
fn main() {
    words := ["now", "is", "the", "time", "for", "all"]

    print(words.map(w -> w.len))
    print(words.filter(w -> w.len == 3))
    print(words.any(w -> w.starts_with("t")))
    print(words.all(w -> w.len < 5))
    print(words.count(w -> w.contains("i")))
    print(words.find(w -> w.len > 3))
    print(words.position(w -> w == "the"))
    print(words.fold(0, (total, w) -> total + w.len))
    print(words.join(" "))
    print(words.enumerate().filter(pair -> pair == (1, "is")))
    print(words.take(2).zip([1, 2]))
}
```

```text
[3, 2, 3, 4, 3, 3]
["now", "the", "for", "all"]
true
true
2
time
2
18
now is the time for all
[(1, "is")]
[("now", 1), ("is", 2)]
```

| | |
| --- | --- |
| `map(f)` | the list of `f(x)` for each element `x` |
| `filter(f)` | the elements for which `f(x)` is true |
| `any(f)`, `all(f)` | whether `f(x)` is true for some element, for every element |
| `count(f)` | how many elements `f(x)` is true for |
| `find(f)` | the first element for which `f(x)` is true, as an optional |
| `position(f)` | the position of that element, as an optional |
| `fold(start, f)` | `f(...f(f(start, x0), x1)..., xn)`: the elements combined, one at a time, into a single value |
| `each(f)` | calls `f(x)` for each element, and returns nothing |
| `join(sep)` | the elements as text, with `sep` between them |
| `enumerate()` | the list of pairs `(position, element)` |
| `zip(other)` | the list of pairs of corresponding elements of two lists, as long as the shorter |

The methods can be chained, each working on the result of the one before:

```koo
// The sum of the squares of the odd numbers below 10.
fn main() {
    var numbers: [int] = []
    for n in 0..10 {
        numbers.push(n)
    }
    print(numbers.filter(n -> n % 2 == 1).map(n -> n * n).sum())
}
```

```text
165
```

Read as it is written, this builds a list of the odd numbers, then a list of their squares, then adds them up. The compiler does better. When the functions in a chain only compute, when they neither print nor change any variable, no program could tell whether the lists in between were ever built, and so the compiler does not build them: the chain is compiled into a single loop over `numbers`. You may write whichever of a chain and a loop reads better, without thinking about speed.

**Exercise 5-2.** Use the methods of this section to write a function that returns the longest string of a list of strings, as an optional. Then write it with a loop. Which do you prefer?

## 5.5 Sorting

`v.sort()` sorts a list in place into increasing order, and `v.sorted()` returns a sorted copy. They apply to lists of numbers and of strings.

To sort by any other rule, give the rule: `sort_by` and `sorted_by` take a function of two elements that says whether the first belongs before the second. The sort is *stable*, which is to say that elements neither of which belongs before the other stay in their original order.

This is enough to write a program that C makes into a major example: sorting a set of text lines. The C version needs an array of pointers to lines, a storage allocator to hold the lines, and a sort function that takes a pointer to a comparison function. Here is the whole program in Koo:

```koo
import std/io

// Sort the input lines.
fn main() {
    var lines := io.read_lines()
    lines.sort()
    for line in lines {
        print(line)
    }
}
```

```input
now is the time
for all good people
to come to the aid
of their party
```

```text
for all good people
now is the time
of their party
to come to the aid
```

`io.read_lines()` reads all of the input, and returns its lines as a list.

Now suppose we want the lines in other orders. Each is one call:

```koo
import std/io

fn main() {
    lines := io.read_lines()

    print("by length:")
    for line in lines.sorted_by((a, b) -> a.len < b.len) {
        print("  {line}")
    }

    print("backwards:")
    for line in lines.sorted_by((a, b) -> a > b) {
        print("  {line}")
    }

    print("by the number at the start:")
    for line in lines.sorted_by((a, b) -> leading_number(a) < leading_number(b)) {
        print("  {line}")
    }
}

// The number that a line begins with, or 0 if it does not begin with one.
fn leading_number(line: string) -> int {
    (first, _) := line.cut(" ") else (line, "")
    return int.parse(first) else 0
}
```

```input
10 green bottles
9 lives
101 dalmatians
```

```text
by length:
  9 lives
  101 dalmatians
  10 green bottles
backwards:
  9 lives
  101 dalmatians
  10 green bottles
by the number at the start:
  9 lives
  10 green bottles
  101 dalmatians
```

`line.cut(" ")` splits a string in two at the first blank and returns the two parts as a tuple, or `none` if there is no blank.

**Exercise 5-3.** Write a program that sorts its input lines without regard to the case of letters, so that `a` and `A` compare equal.

**Exercise 5-4.** Write the program `tail`, which prints the last 10 lines of its input.

## 5.6 Strings

A string is text. Like a list it is a value: it can be assigned, passed, returned and compared without any thought for storage. Unlike a list it cannot be subscripted, and the reason is the encoding.

The text of a string is stored in UTF-8, in which a character occupies from one to four bytes. The characters of English take one byte each; an accented letter usually takes two; a Chinese character takes three. So "the fifth character" is not at any fixed place, and a subscript that looked cheap would be either slow or wrong. Koo makes you say which you mean:

- `s.len` is the number of bytes, and `s.count()` the number of characters;
- `s.bytes()` is the bytes, as a list of `byte`, and `s.chars()` the characters, as a list of strings of one character each;
- `s.slice(from, to)` is the part of the string from byte `from` up to but not including byte `to`.

The positions that string methods return, such as that of `find`, are byte positions, and are meant to be handed back to `slice`. A position that is in the middle of a character, or beyond the end, is a panic.

```koo
fn main() {
    s := "naïve café"
    print("{s.len} bytes, {s.count()} characters")
    print(s.chars())
    if at := s.find("café") {
        print("found at byte {at}: {s.slice(at, s.len)}")
    }
    print(s.to_upper())
}
```

```text
12 bytes, 10 characters
["n", "a", "ï", "v", "e", " ", "c", "a", "f", "é"]
found at byte 7: café
NAÏVE CAFÉ
```

Every string is valid UTF-8, always. The functions that make strings out of bytes from outside the program check them.

Here are the methods for looking into a string and taking it apart:

```koo
fn main() {
    line := "  name = Grace Hopper  "
    text := line.trim()

    print(text.starts_with("name"))
    print(text.ends_with("x"))
    print(text.contains("Grace"))
    print(text.find("="))
    print(text.cut("="))
    print(text.split(" "))
    print(text.strip_prefix("name = "))
    print(text.strip_prefix("title"))
    print("a,b,,c".split(","))
    print("one\ntwo\nthree".lines())
}
```

```text
true
false
true
5
("name ", " Grace Hopper")
["name", "=", "Grace", "Hopper"]
Grace Hopper
none
["a", "b", "", "c"]
["one", "two", "three"]
```

`trim` removes white space from both ends. `cut(sep)` splits at the first occurrence of `sep`, into the part before and the part after, and `split(sep)` at every occurrence. `strip_prefix` is the string without the given beginning, or `none` if it does not begin so; there is a `strip_suffix` to match. `lines` splits a text at its newlines.

And these make new strings:

```koo
fn main() {
    name := "koo"
    print(name.to_upper())
    print("KOO".to_lower())
    print(name.replace("o", "0"))
    print(name.repeat(3))
    print(name + "!" + name)

    var out := ""
    for word in ["a", "b", "c"] {
        if not out.is_empty() {
            out += ", "
        }
        out += word
    }
    print(out)
}
```

```text
KOO
koo
k00
kookookoo
koo!koo
a, b, c
```

Adding to the end of a string with `+=` is efficient: the string has room to grow, and is not copied each time.

As an example of string handling, here is a function that tells whether a string reads the same in both directions, ignoring everything but its letters and digits. It is not as simple as comparing the string with its reverse, because of the ignoring, and it shows the usual way of working with characters: take the string apart with `chars`, work on the list, and put a string together again if one is wanted.

```koo
// Whether s reads the same forwards and backwards, letters and digits only.
fn is_palindrome(s: string) -> bool {
    letters := s.to_lower().chars().filter(c -> (c >= "a" and c <= "z") or (c >= "0" and c <= "9"))
    for i in 0..letters.len / 2 {
        if letters[i] != letters[letters.len - 1 - i] {
            return false
        }
    }
    return true
}

fn main() {
    print(is_palindrome("A man, a plan, a canal: Panama"))
    print(is_palindrome("Not a palindrome"))
}
```

```text
true
false
```

**Exercise 5-5.** Write a function `capitalize(s)` that returns `s` with the first letter of each word in upper case.

**Exercise 5-6.** Write a function `str_end(s, t)`, which returns `true` if the string `t` occurs at the end of the string `s`, without using `ends_with`.

## 5.7 Lists of Lists

Koo provides rectangular arrays in the only way it provides anything: a list whose elements are lists. Consider the problem of date conversion, from day of the month to day of the year and vice versa. For example, March 1 is the 60th day of a non-leap year, and the 61st day of a leap year. Let us define two functions to do the conversions: `day_of_year` converts the month and day into the day of the year, and `month_day` converts the day of the year into the month and day. Since this latter function computes two values, it returns a tuple.

These functions both need the same information, a table of the number of days in each month. Since the number of days per month differs for leap years and non-leap years, it is easier to separate them into two rows of a two-dimensional list than to keep track of what happens to February during computation.

```koo
// The days in each month, in an ordinary year and in a leap year.
fn day_table() -> [[int]] {
    return [
        [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31],
        [0, 31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31],
    ]
}

fn is_leap(year: int) -> bool {
    return (year % 4 == 0 and year % 100 != 0) or year % 400 == 0
}

// The day of the year, from the month and the day of the month.
fn day_of_year(year: int, month: int, day: int) -> int {
    days := day_table()[if is_leap(year) { 1 } else { 0 }]
    var total := day
    for m in 1..month {
        total += days[m]
    }
    return total
}

// The month and the day of the month, from the day of the year.
fn month_day(year: int, year_day: int) -> (int, int) {
    days := day_table()[if is_leap(year) { 1 } else { 0 }]
    var left := year_day
    var month := 1
    while left > days[month] {
        left -= days[month]
        month += 1
    }
    return (month, left)
}

fn main() {
    print(day_of_year(1988, 3, 1))
    print(day_of_year(1989, 3, 1))
    (month, day) := month_day(1988, 60)
    print("{month}/{day}")
}
```

```text
61
60
2/29
```

The table is returned by a function, because a constant at the top level must be a simple value and a list is not one. The list has a leading 0 in each row so that month numbers run from 1 to 12, as is natural, and not from 0 to 11. `day_table()[1]` is a row, which is a list of `int`, and `days[m]` an element of it; an element of the whole table is `table[row][column]`.

The rows of such a list are lists in their own right, and need not be of the same length.

A list of strings does the job that an array of character pointers does in C. Here is the function `month_name(n)`, which returns the name of the `n`-th month:

```koo
// The name of the n-th month.
fn month_name(n: int) -> string {
    names := [
        "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December",
    ]
    return names.get(n - 1) else "illegal month"
}

fn main() {
    print(month_name(3))
    print(month_name(13))
}
```

```text
March
illegal month
```

**Exercise 5-7.** There is no error checking in `day_of_year` or `month_day`. Remedy this defect. What should each return for a date that does not exist?

## 5.8 Maps

A *map* associates values with keys. Where a list finds an element by its position, a map finds one by its key, which may be a string, a number, or a value of nearly any other type. The type of a map from keys of type `K` to values of type `V` is written `[K: V]`. A map is written as pairs of key and value between brackets, and the empty map as `[:]`.

```koo
fn main() {
    var stock := ["pens": 120, "ink": 8]
    stock["paper"] = 500
    stock["ink"] = 7

    print(stock["pens"])
    print(stock["glue"])
    print(stock["glue"] else 0)
    print(stock.len)
    print(stock.contains("ink"))
    print(stock.remove("pens"))
    print(stock)
    print(stock.keys())
    print(stock.values())
}
```

```text
120
none
0
3
true
120
["ink": 7, "paper": 500]
["ink", "paper"]
[7, 500]
```

`m[key] = value` adds an entry or replaces the one with that key. `m[key]`, as an expression, is an *optional*: the value, or `none` if the map has no such key. This is the map's version of the checked subscript. In a language where a missing key gives back zero, or an empty string, or a crash, every lookup is a small bet; here the type will not let you forget that the key may be missing, and `else` says what to do about it in three words.

A map remembers the order in which its keys were first added, and a `for` loop over it gives the entries in that order, each as a pair. The same program therefore prints the same thing every time it is run, which is not true of the hash tables of most languages.

The classic use of a map is to count things. This program counts the occurrences of each word in its input, and prints the words in decreasing order of frequency:

```koo
import std/io

// Count the occurrences of each word in the input.
fn main() {
    var counts: [string: int] = [:]
    while line := io.read_line() {
        for word in line.to_lower().split(" ") {
            if word != "" {
                counts.update(word, 0, n -> n + 1)
            }
        }
    }
    words := counts.keys().sorted_by((a, b) -> (counts[a] else 0) > (counts[b] else 0))
    for word in words {
        print("{counts[word] else 0:4} {word}")
    }
}
```

```input
the quick brown fox
jumps over the lazy dog
The dog barks
```

```text
   3 the
   2 dog
   1 quick
   1 brown
   1 fox
   1 jumps
   1 over
   1 lazy
   1 barks
```

`counts.update(key, start, f)` replaces the value for `key` by `f` of it. If there is no such key, it begins from `start`: so the first time a word is seen its count becomes `f(0)`, which is 1. The compound assignment operators cannot be used on a map entry, precisely because the entry might not be there, and `update` is what to use in their place.

The words that have the same count come out in the order in which they first appeared. This is because the sort is stable, and the map gave the keys in the order they were added.

The keys of a map may be of any type that can be compared for equality and *hashed*, that is, reduced to a number: integers, strings, booleans, bytes, and lists, tuples, structures and enumerations made of these. A `float` cannot be a key, nor can anything that contains one.

```koo
// Mark squares of a board, and say which are marked.
fn main() {
    var marked: [(int, int): string] = [:]
    marked[(0, 0)] = "X"
    marked[(1, 1)] = "O"
    for row in 0..2 {
        for column in 0..2 {
            print("({row}, {column}): {marked[(row, column)] else "."}")
        }
    }
}
```

```text
(0, 0): X
(0, 1): .
(1, 0): .
(1, 1): O
```

A map is a value, like everything else: assigning it gives an independent map, and a function changes the caller's map only through a `var` parameter.

**Exercise 5-8.** Write a program that prints the distinct words of its input, sorted alphabetically, each followed by the numbers of the lines on which it occurs.

## 5.9 Sets

A *set* is a collection in which each value appears at most once. Its type is `Set<T>`. A set is made empty by `Set<T>.new()`, or from a list by `Set<T>.from_list(list)`.

```koo
fn main() {
    var seen := Set<string>.new()
    for word in ["to", "be", "or", "not", "to", "be"] {
        if not seen.insert(word) {
            print("{word} again")
        }
    }
    print(seen.len)
    print(seen.contains("or"))
    print(seen.remove("or"))
    print(seen.items())
    print(seen)
}
```

```text
to again
be again
4
true
true
["to", "be", "not"]
Set["to", "be", "not"]
```

`insert` returns whether the value was new, and `remove` whether it was there, which often saves a separate test. `items()` is the members as a list, in the order they were added, and a `for` loop over a set runs in that order too. The members of a set are of a type that could be the key of a map.

## 5.10 Tuples

A *tuple* is a few values grouped together, possibly of different types. It is written, and its type is written, with parentheses: `(3, "three")` is of type `(int, string)`.

The elements of a tuple have no names and no numbers. A tuple is taken apart by a *pattern* of the same shape, which names the parts that are wanted, with `_` for those that are not. A pattern may be used in a declaration, in a `for` loop, in a binding condition, and in the arms of a `match`.

```koo
// The smallest and the largest of a list of numbers, which is not empty.
fn min_max(numbers: [int]) -> (int, int) {
    var low := numbers[0]
    var high := numbers[0]
    for n in numbers {
        if n < low {
            low = n
        }
        if n > high {
            high = n
        }
    }
    return (low, high)
}

fn main() {
    (low, high) := min_max([4, -2, 9, 0])
    print("from {low} to {high}")

    (_, top) := min_max([1, 2, 3])
    print(top)

    pairs := [("one", 1), ("two", 2)]
    for (name, value) in pairs {
        print("{name} = {value}")
    }

    if (key, value) := "port=8080".cut("=") {
        print("{key} is {value}")
    }
}
```

```text
from -2 to 9
3
one = 1
two = 2
port is 8080
```

Tuples are for the short term: the two results of a function, the key and value of an entry. When a group of values has a meaning of its own, and travels far through a program, it deserves a name and names for its parts, and should be a structure.

## 5.11 Optionals

We have used optionals since Chapter 1. Here is the whole of them.

For any type `T`, the type `T?` is "a `T`, or nothing". Its values are all the values of `T`, and `none`. A `T` may be used wherever a `T?` is wanted, but not the other way round: before a `T?` can be used as a `T`, the program must deal with the possibility that it is `none`. There are five ways to do it.

```koo
fn find(names: [string], prefix: string) -> string? {
    for name in names {
        if name.starts_with(prefix) {
            return name
        }
    }
    return none
}

fn main() {
    names := ["ada", "grace", "edsger"]

    // 1. Give a value to use in its place.
    print(find(names, "x") else "nobody")

    // 2. Test for it, and name it if it is there.
    if name := find(names, "gr") {
        print("found {name}")
    }

    // 3. Leave if it is not there.
    name := find(names, "ed") else {
        print("not found")
        return
    }
    print(name.to_upper())

    // 4. Ask only whether it is there.
    print(find(names, "a").is_some())
    print(find(names, "z").is_none())

    // 5. Take both cases in a match.
    match find(names, "q") {
        none => print("no q")
        found => print("found {found}")
    }

    // And map applies a function to the value, if there is one.
    print(find(names, "ad").map(n -> n.len))
    print(find(names, "zz").map(n -> n.len))
}
```

```text
nobody
found grace
EDSGER
true
true
no q
3
none
```

`print` writes an optional as its value if it has one, and as `none` if not.

Many functions in other languages return a special value to say "no answer": −1, an empty string, a null pointer. Each is a small trap, since the special value is of the same type as a real answer and will be used as one by any caller who forgets to test for it. In Koo a function that may have no answer returns an optional, and the trap cannot be sprung, because the forgetful program does not compile.

```koo
fn main() {
    names := ["ada", "grace"]
    first := names.first()
    print(first.len)
}
```

```error
error: the value may be absent, so `.len` cannot be used on it
 --> main.koo:4:11
  |
4 |     print(first.len)
  |           ^^^^^ this is a `string?`: it may be `none`
  |
  = help: give a value to use when there is none, `(x else fallback).len`, or use it only when it is there, `if value := x { ... value.len ... }` (Section 4.8)

1 error
```
