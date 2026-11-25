# Koo for Visual Studio Code

Syntax highlighting for `.koo` files: comments, strings with their escapes and the
expressions inside them, numbers, keywords, types and the names of declarations.

It is not published yet. To use it, link or copy this directory into your extensions:

```sh
ln -s "$PWD/editors/vscode" ~/.vscode/extensions/koo-language.koo-0.0.1
```

and restart the editor. There is no language server yet: errors come from `koo check`.
