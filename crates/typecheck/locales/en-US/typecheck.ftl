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

already-defined = the name `{$name}` is defined multiple times
already-defined-original = previously defined here
