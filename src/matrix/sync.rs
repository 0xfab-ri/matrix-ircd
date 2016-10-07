// Copyright 2016 Openmarket
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use futures::{self, Async, Future, Poll};
use futures::stream::Stream;

use serde_json;

use std::{io, str};

use super::protocol::SyncResponse;

use tokio_core::reactor::Handle;

use url::Url;

use http::{Request, Response, HttpStream};


pub struct MatrixSyncClient {
    url: Url,
    access_token: String,
    next_token: Option<String>,
    http_stream: HttpStream,
    current_sync: Option<futures::Oneshot<Response>>,
}

impl MatrixSyncClient {
    pub fn new(handle: Handle, base_url: &Url, access_token: String) -> MatrixSyncClient {
        let host = base_url.host_str().expect("expected host in base_url");
        let port = base_url.port_or_known_default().unwrap();

        MatrixSyncClient {
            url: base_url.join("/_matrix/client/r0/sync").unwrap(),
            access_token: access_token,
            next_token: None,
            http_stream: HttpStream::new(host.into(), port, handle),
            current_sync: None,
        }
    }

    pub fn poll_sync(&mut self) -> Poll<SyncResponse, io::Error> {
        loop {
            let sync_response: SyncResponse = {
                let mut http_stream = &mut self.http_stream;

                let current_sync = if let Some(ref mut current_sync) = self.current_sync {
                    current_sync
                } else {
                    self.url
                        .query_pairs_mut()
                        .clear()
                        .append_pair("access_token", &self.access_token)
                        .append_pair("filter", r#"{"presence":{"not_types":["m.presence"]}}"#)
                        .append_pair("timeout", "30000");

                    if let Some(ref token) = self.next_token {
                        self.url.query_pairs_mut().append_pair("since", token);
                    }

                    self.current_sync = Some(http_stream.send_request(&Request {
                        method: "GET",
                        path: &format!("{}?{}", self.url.path(), self.url.query().unwrap_or("")),
                        headers: &[],
                        body: &[],
                    }));
                    continue
                };

                http_stream.poll()?;

                let response = match current_sync.poll().expect("sync future unexpectedly cancelled") {
                    Async::Ready(r) => r,
                    Async::NotReady => return Ok(Async::NotReady),
                };

                if response.code != 200 {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Sync returned {}", response.code)));
                }

                serde_json::from_slice(&response.data).map_err(|e| {
                    io::Error::new(io::ErrorKind::Other, format!("Sync returned invalid JSON: {}", e))
                })?
            };

            task_trace!("Got sync response"; "next_token" => sync_response.next_batch);

            self.next_token = Some(sync_response.next_batch.clone());

            return Ok(Async::Ready(sync_response))
        }
    }
}

impl Stream for MatrixSyncClient {
    type Item = SyncResponse;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<SyncResponse>, io::Error> {
        task_trace!("Matrix Sync Polled");

        let res = try_ready!(self.poll_sync());
        Ok(Async::Ready(Some(res)))
    }
}
