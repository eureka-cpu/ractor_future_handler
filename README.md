# Ractor Future Handler

This is a minimal reproduction of how to forward futures out of the `Actor::handle` method in the `ractor` crate.
The `handle` method when called internally, blocks the actor until `handle` completes, but this is inefficient 
when an actor could pass on the `Future`s that are `await`ed in the `handle` method, to be `await`ed elsewhere.

This strategy relies on that the `ractor::Actor::State` is a global mutable variable, and that the actor itself contains
a field for storing `Future`s. Once each `Future` is passed to the future pool on the actor, they can then
be polled for the next available `Future` that is ready and be passed to a `rayon::ThreadPool` to be `await`ed.

To see this implementation in action, check out the Versatus `lasr` repository: https://github.com/versatus/lasr
