; diffr additions (MIT); later patterns override the grammar's.
(keyword_null) @constant.builtin
[
  (keyword_true)
  (keyword_false)
] @boolean
((literal) @number
  (#match? @number "^[-+]?[0-9.]"))
[
  (keyword_asc)
  (keyword_desc)
] @keyword
