; Written for diffr (MIT). Scripts and styles are named by their start tag,
; whose lang attribute `injection_language` reads; expressions are
; JavaScript.
((script_element
  (start_tag) @injection.language
  (raw_text) @injection.content))

((style_element
  (start_tag) @injection.language
  (raw_text) @injection.content))

((svelte_raw_text) @injection.content
  (#set! injection.language "javascript"))
