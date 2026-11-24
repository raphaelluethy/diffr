; diffr additions (MIT); later patterns override the grammar's.
; self and cls, as Chroma's builtin pseudo-names.
((identifier) @variable.builtin
 (#any-of? @variable.builtin "self" "cls"))

; Builtin exceptions.
((identifier) @exception
 (#match? @exception "^(BaseException|Exception|ArithmeticError|AssertionError|AttributeError|EOFError|ImportError|IndexError|KeyError|LookupError|MemoryError|NameError|NotImplementedError|OSError|OverflowError|RecursionError|RuntimeError|StopIteration|StopAsyncIteration|SyntaxError|SystemError|SystemExit|TypeError|ValueError|ZeroDivisionError|KeyboardInterrupt|GeneratorExit|FileNotFoundError|PermissionError|TimeoutError|ConnectionError|UnicodeError|UnicodeDecodeError|UnicodeEncodeError|ModuleNotFoundError|[A-Za-z]*Warning)$"))
