; diffr additions (MIT); later patterns override the grammar's.
; Variables in strings are not string, as in Chroma.
(simple_expansion) @embedded
(simple_expansion "$" @embedded)
(expansion "${" @embedded "}" @embedded)

[
  "local"
  "export"
  "declare"
  "readonly"
  "typeset"
  "unset"
] @keyword

(command
  name: (command_name) @keyword
  (#any-of? @keyword "return" "exit" "break" "continue" "shift" "source" "eval" "exec"))
