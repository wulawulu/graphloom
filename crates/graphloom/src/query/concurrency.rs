//! Bounded, fail-fast Query concurrency helpers.

use std::future::Future;

use futures_util::{StreamExt, TryStreamExt, stream};

pub(crate) async fn try_buffered_ordered<I, F, T, E>(
    futures: I,
    concurrency: usize,
) -> std::result::Result<Vec<T>, E>
where
    I: IntoIterator<Item = F>,
    F: Future<Output = std::result::Result<T, E>>,
{
    let indexed = futures
        .into_iter()
        .enumerate()
        .map(|(index, future)| async move { future.await.map(|value| (index, value)) });
    let mut completed = stream::iter(indexed)
        .buffer_unordered(concurrency.max(1))
        .try_collect::<Vec<_>>()
        .await?;
    completed.sort_unstable_by_key(|(index, _)| *index);
    Ok(completed.into_iter().map(|(_, value)| value).collect())
}

#[cfg(test)]
mod tests {
    use std::{
        future,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use futures_util::future::BoxFuture;

    use super::try_buffered_ordered;

    #[tokio::test]
    async fn test_should_bound_concurrency_and_restore_input_order() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let futures = (0..8).map(|index| {
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            async move {
                let current = active.fetch_add(1, Ordering::SeqCst).saturating_add(1);
                maximum.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(
                    u64::try_from(8_usize.saturating_sub(index)).unwrap_or(0),
                ))
                .await;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok::<_, ()>(index)
            }
        });

        let values = try_buffered_ordered(futures, 3)
            .await
            .expect("bounded results");

        assert_eq!(values, (0..8).collect::<Vec<_>>());
        assert!(maximum.load(Ordering::SeqCst) <= 3);
    }

    #[tokio::test]
    async fn test_should_return_fast_error_without_waiting_for_earlier_slow_future() {
        let futures: Vec<BoxFuture<'static, std::result::Result<(), &'static str>>> = vec![
            Box::pin(future::pending()),
            Box::pin(async { Err("fast failure") }),
        ];

        let result =
            tokio::time::timeout(Duration::from_millis(100), try_buffered_ordered(futures, 2))
                .await
                .expect("fail-fast helper must not wait for pending work");

        assert_eq!(result, Err("fast failure"));
    }
}
