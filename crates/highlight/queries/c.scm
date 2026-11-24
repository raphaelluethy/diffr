; diffr additions (MIT); later patterns override the grammar's.
; Preprocessor directives are preproc, as in Chroma.
[
  "#include"
  "#define"
  "#if"
  "#ifdef"
  "#ifndef"
  "#else"
  "#elif"
  "#endif"
  "#elifdef"
  "#elifndef"
] @keyword.directive
(preproc_directive) @keyword.directive

(char_literal) @string
[(true) (false)] @boolean
(null) @constant.builtin
