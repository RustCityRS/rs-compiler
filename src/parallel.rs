/// Map `f` over `items` across worker threads, returning results in the SAME
/// order as the input. Dependency-free (uses `std::thread::scope`); each item
/// is processed independently, so callers must keep the work pure (no shared
/// mutable state). Falls back to a serial map for tiny inputs or single-core.
pub(crate) fn parallel_map<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    if threads <= 1 || items.len() <= 1 {
        return items.iter().map(&f).collect();
    }
    let threads = threads.min(items.len());
    let chunk_size = items.len().div_ceil(threads);
    let mut out: Vec<R> = Vec::with_capacity(items.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk_size)
            .map(|chunk| {
                let f = &f;
                scope.spawn(move || chunk.iter().map(f).collect::<Vec<R>>())
            })
            .collect();
        // Joining in spawn order concatenates chunks in input order, so the
        // returned Vec matches a serial map element-for-element.
        for h in handles {
            out.extend(h.join().expect("worker thread panicked"));
        }
    });
    out
}

/// Like `parallel_map`, but hands each worker a whole contiguous CHUNK of items
/// so per-chunk setup (e.g. a checker's caches) can be reused across the chunk.
/// Returns one result per chunk, in input order.
pub(crate) fn parallel_chunks<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&[T]) -> R + Sync,
{
    parallel_chunks_capped(items, usize::MAX, f)
}

/// `parallel_chunks` with an upper bound on the number of concurrent chunks.
/// Fewer chunks → less peak memory when each chunk holds large per-chunk scratch
/// (e.g. the pointer checker's CFG caches), at the cost of less parallelism.
/// Result is independent of the chunk count, so output stays byte-identical.
pub(crate) fn parallel_chunks_capped<T, R, F>(items: &[T], max_chunks: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&[T]) -> R + Sync,
{
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(max_chunks.max(1));
    if threads <= 1 || items.len() <= 1 {
        return vec![f(items)];
    }
    let threads = threads.min(items.len());
    let chunk_size = items.len().div_ceil(threads);
    let mut out: Vec<R> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk_size)
            .map(|chunk| {
                let f = &f;
                scope.spawn(move || f(chunk))
            })
            .collect();
        for h in handles {
            out.push(h.join().expect("worker thread panicked"));
        }
    });
    out
}
