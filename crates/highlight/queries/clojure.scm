; diffr additions (MIT); later patterns override the grammar's.
(kwd_lit) @string.special.symbol
(regex_lit) @string.regex
(bool_lit) @boolean
(nil_lit) @constant.builtin

; The head of a list is called.
(list_lit
  .
  (sym_lit) @function)

; Special forms and core macros are keywords.
(list_lit
  .
  (sym_lit) @keyword
  (#match? @keyword "^(def|defn|defn-|defmacro|defmulti|defmethod|defprotocol|defrecord|deftype|definterface|definline|defonce|defstruct|ns|fn|fn\\*|if|if-not|if-let|if-some|when|when-not|when-let|when-some|when-first|cond|condp|cond->|cond->>|case|do|let|letfn|loop|recur|binding|try|catch|finally|throw|quote|var|new|set!|monitor-enter|monitor-exit|and|or|doseq|dotimes|for|while|->|->>|some->|some->>|as->|doto|import|require|use|refer|declare|delay|lazy-seq|reify|proxy|extend-type|extend-protocol)$"))

; What a definition defines is a function. Last, as a later pattern
; matching the keyword would drop this match.
(list_lit
  .
  (sym_lit) @keyword
  .
  (sym_lit) @function
  (#match? @keyword "^(defn|defn-|defmacro|defmulti|defmethod|definline)$"))
