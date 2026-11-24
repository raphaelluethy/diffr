; Written for diffr (MIT). Scripts and styles are named by their start tag,
; whose lang attribute `injection_language` reads; interpolations and
; directive values are JavaScript.
((script_element
  (start_tag) @injection.language
  (raw_text) @injection.content))

((style_element
  (start_tag) @injection.language
  (raw_text) @injection.content))

((interpolation
  (raw_text) @injection.content)
  (#set! injection.language "javascript"))

((directive_attribute
  (quoted_attribute_value
    (attribute_value) @injection.content))
  (#set! injection.language "javascript"))
