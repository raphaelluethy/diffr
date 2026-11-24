; Written for diffr (MIT): the grammar crate ships no highlights query.
[
  (comments)
  (pod_statement)
  (data_not_for_compiler)
] @comment

; Strings and quote-like operators, variables in them included, as in
; Chroma.
[
  (string_single_quoted)
  (string_double_quoted)
  (string_q_quoted)
  (string_qq_quoted)
  (command_qx_quoted)
  (backtick_quoted)
  (word_list_qw)
  (heredoc_body_statement)
  (heredoc_initializer)
] @string
(escape_sequence) @string.escape

[
  (pattern_matcher)
  (pattern_matcher_m)
  (regex_pattern_qr)
  (substitution_pattern_s)
  (transliteration_tr_or_y)
] @string.regex

[
  (integer)
  (floating_point)
  (hexadecimal)
  (octal)
  (scientific_notation)
  (version)
] @number
(special_literal) @constant.builtin

(function_definition name: (identifier) @function)
(call_expression_with_bareword function_name: (identifier) @function)
(method_invocation function_name: (identifier) @function)

[
  "BEGIN"
  "CHECK"
  "END"
  "INIT"
  "UNITCHECK"
  "__DATA__"
  "__END__"
  "and"
  "bless"
  "cmp"
  "continue"
  "else"
  "elsif"
  "eq"
  "for"
  "foreach"
  "ge"
  "goto"
  "gt"
  "if"
  "isa"
  "last"
  "le"
  "local"
  "lt"
  "my"
  "ne"
  "next"
  "no"
  "not"
  "or"
  "our"
  "package"
  "redo"
  "require"
  "return"
  "state"
  "sub"
  "unless"
  "until"
  "use"
  "when"
  "while"
  "xor"
] @keyword

[
  "=~"
  "!~"
  "=="
  "!="
  "<=>"
  "<="
  ">="
  "&&"
  "||"
  "//"
  "!"
  "="
  "+"
  "-"
  "*"
  "/"
  "**"
  ".="
  "+="
  "-="
  "->"
  "?"
] @operator

; The slashes of /…/ are the regex's, not division.
(pattern_matcher
  "/" @string.regex)
