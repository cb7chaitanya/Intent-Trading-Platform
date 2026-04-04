use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::Request;
use axum::response::Response;
use futures_util::future::BoxFuture;
use tower::{Layer, Service};

use crate::metrics::{counters, histograms};

#[derive(Clone)]
pub struct MetricsLayer;

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService { inner }
    }
}

#[derive(Clone)]
pub struct MetricsService<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for MetricsService<S>
where
    S: Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
        let start = std::time::Instant::now();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let response = inner.call(req).await?;

            let status = response.status().as_u16().to_string();
            let elapsed = start.elapsed().as_secs_f64();

            counters::API_REQUESTS_TOTAL
                .with_label_values(&[&method, &path, &status])
                .inc();
            histograms::API_REQUEST_DURATION
                .with_label_values(&[&method, &path])
                .observe(elapsed);

            Ok(response)
        })
    }
}
