; Written for diffr (MIT): the grammar crate ships no highlights query.
(comment) @comment
(description) @string

(string_value) @string
[
  (int_value)
  (float_value)
] @number
(boolean_value) @boolean
(null_value) @constant.builtin
(enum_value) @constant
(directive_location) @constant

(named_type (name) @type)
(object_type_definition (name) @type)
(interface_type_definition (name) @type)
(enum_type_definition (name) @type)
(input_object_type_definition (name) @type)
(union_type_definition (name) @type)
(scalar_type_definition (name) @type)
(object_type_extension (name) @type)
(interface_type_extension (name) @type)
(enum_type_extension (name) @type)
(input_object_type_extension (name) @type)
(union_type_extension (name) @type)
(scalar_type_extension (name) @type)

(field_definition (name) @property)
(input_value_definition (name) @property)
(operation_definition (name) @function)
(fragment_name (name) @function)
(directive "@" @attribute (name) @attribute)
(directive_definition "@" @attribute (name) @attribute)

[
  "query"
  "mutation"
  "subscription"
  "fragment"
  "on"
  "type"
  "interface"
  "enum"
  "input"
  "union"
  "scalar"
  "schema"
  "extend"
  "implements"
  "directive"
  "repeatable"
] @keyword

[
  "!"
  "="
  "|"
  "&"
] @operator
