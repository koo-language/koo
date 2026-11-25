# What a `koo` binary carries of others' work

Koo is licensed as [LICENSE](LICENSE) says. A built `koo` also carries code written by others, and links it into the programs it builds.

## musl

A `koo` that builds for the targets `linux-arm64` and `linux-x86-64` carries the C library [musl](https://musl.libc.org), as the Rust toolchain distributes it for its `-musl` targets, and links it into every program built with `--target`. musl as a whole is under the MIT license:

```text
Copyright © 2005-2020 Rich Felker, et al.

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

musl's own `COPYRIGHT` file names the authors of its parts and the few files under other permissive licenses.

## The Rust libraries

The support library that is linked into every Koo program is written in Rust, and contains Rust's `core` and `compiler_builtins`, which are under the MIT license or the Apache License 2.0, at the user's choice.

## LLVM

The start-up files `crtbegin.o` and `crtend.o` in a program built with `--target` are LLVM's, under the Apache License 2.0 with LLVM exceptions.
