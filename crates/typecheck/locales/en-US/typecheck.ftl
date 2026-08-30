type-mismatch = expected `{$expected}`, found `{$found}`
type-mismatch-expected-due-to = expected due to this
type-mismatch-expected-because-of = expected because of this
type-mismatch-generic-note = `{$name}` is generic here and must work for every type, not just `{$other}`
type-mismatch-provenance = {$side} `{$kind}` was inferred here

cyclic-type = cyclic type of infinite size
cyclic-type-expected = expected type `{$expected}`
cyclic-type-found = found type `{$found}`

argument-count-mismatch =
    this function takes { $expected ->
        [one] {$expected} argument
       *[other] {$expected} arguments
    } but { $found ->
        [one] {$found} argument
       *[other] {$found} arguments
    } { $found ->
        [one] was
       *[other] were
    } supplied

generic-argument-count-mismatch =
    expected { $expected ->
        [one] {$expected} generic argument
       *[other] {$expected} generic arguments
    }, found {$found}

not-callable = expected a function, found `{$found}`

annotations-needed = type annotations needed

unresolved-import = unresolved import `{$path}`

invalid-glob-target = cannot glob-import `{$path}`: expected a module, found {$found}

unresolved-type = cannot find type `{$path}` in this scope

unresolved-value = cannot find value `{$path}` in this scope

already-defined = the symbol `{$name}` is defined multiple times
already-defined-original = previously defined here

unknown-field = struct `{$struct_name}` has no field named `{$name}`

missing-field = missing field `{$name}` in initializer of `{$struct_name}`

invalid-tuple-index = no field `{$name}` on type `{$found}`

tuple-index-out-of-bounds = index out of bounds: the tuple `{$found}` has { $len ->
    [one] {$len} element
   *[other] {$len} elements
} but the index is {$index}

invalid-field-access = expected a struct or tuple, found `{$found}`

missing-trait-item = missing {$kind} `{$name}` from trait `{$trait_name}`

missing-self-param = `{$name}` is missing a `self` parameter required by trait `{$trait_name}`
missing-self-param-declared-here = `self` is declared here

unexpected-self-param = `{$name}` has a `self` parameter, but trait `{$trait_name}` does not declare one
unexpected-self-param-declared-here = declared without `self` here

self-outside-impl-or-trait = `self` parameter is only valid inside an `impl` or `trait` block
