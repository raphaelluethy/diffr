; diffr additions (MIT); later patterns override the grammar's.
; As in Chroma: syntax is a keyword, its version a string.
[
  "syntax"
  "edition"
] @keyword
(syntax
  version: (string) @string)
