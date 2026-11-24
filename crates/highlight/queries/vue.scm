; diffr additions (MIT), after tree-sitter-html's query, whose node names
; the Vue grammar shares.
; Directives (v-if, :prop, @event, #slot) are attributes.
(directive_name) @attribute
(directive_value) @attribute
(directive_modifier) @attribute
(dynamic_directive_value) @attribute
(directive_attribute
  [
    ":"
    "@"
    "#"
  ] @attribute)
