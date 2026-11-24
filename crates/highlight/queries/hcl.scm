; Written for diffr (MIT): the grammar crate ships no highlights query.
(comment) @comment

; Block types (resource, variable, …) and their labels.
(block (identifier) @keyword)
(block (string_lit) @string)

(attribute (identifier) @property)
(object_elem key: (expression (variable_expr (identifier) @property)))

[
  (string_lit)
  (quoted_template)
  (heredoc_template)
] @string
[
  (heredoc_start)
  (heredoc_identifier)
] @string

; Interpolations are code.
(template_interpolation) @embedded
(template_directive) @embedded

(numeric_lit) @number
(bool_lit) @boolean
(null_lit) @constant.builtin

(function_call (identifier) @function)

[
  "if"
  "else"
  "endif"
  "for"
  "endfor"
  "in"
] @keyword

[
  "!"
  "!="
  "%"
  "&&"
  "*"
  "+"
  "-"
  "/"
  "<"
  "<="
  "=="
  ">"
  ">="
  "?"
  "||"
  "=>"
  "="
  ":"
] @operator
