; Written for diffr (MIT): the grammar crate ships no highlights query.
[
  (line_comment)
  (block_comment)
] @comment
(shebang) @preproc

[
  (string_literal)
  (character_literal)
] @string
(escape_sequence) @string.escape
; Interpolations are code.
(string_interpolation) @embedded

[
  (decimal_integer_literal)
  (hex_integer_literal)
  (octal_integer_literal)
  (binary_integer_literal)
  (decimal_floating_point_literal)
  (hex_floating_point_literal)
] @number
[
  (true)
  (false)
] @boolean
(null_literal) @constant.builtin
[
  (this)
  (super)
] @variable.builtin

(type_identifier) @type
[
  (integral_type)
  (floating_point_type)
  (boolean_type)
  (void_type)
] @type.builtin
(class_declaration name: (identifier) @type)

(method_declaration name: (identifier) @function)
(function_definition name: (identifier) @function)
(method_invocation name: (identifier) @function)
(juxt_function_call name: (identifier) @function)

(annotation "@" @attribute name: (_) @attribute)
(marker_annotation "@" @attribute name: (_) @attribute)

[
  "abstract"
  "as"
  "assert"
  "break"
  "case"
  "catch"
  "class"
  "continue"
  "def"
  "default"
  "do"
  "else"
  "enum"
  "extends"
  "final"
  "finally"
  "for"
  "if"
  "implements"
  "import"
  "in"
  "instanceof"
  "interface"
  "native"
  "new"
  "package"
  "permits"
  "private"
  "protected"
  "public"
  "record"
  "return"
  "sealed"
  "static"
  "strictfp"
  "switch"
  "synchronized"
  "throw"
  "throws"
  "transient"
  "try"
  "volatile"
  "while"
  "yield"
] @keyword

[
  "="
  "=="
  "!="
  "<"
  "<="
  ">"
  ">="
  "+"
  "-"
  "*"
  "/"
  "%"
  "**"
  "!"
  "&&"
  "||"
  "?"
  ":"
  "->"
  ".."
  "+="
  "-="
  "++"
  "--"
] @operator
