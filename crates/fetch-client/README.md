# cfa4-fetch-client

The engine's side of downloads ([CFA4-SECURITY-001](../../docs/plans/CFA4-SECURITY-001.md),
decision D5). The engine opens no connection to the internet: a download the user asked for (a
speech model, the natural voices) runs in the `cfa4-fetch` program (`apps/cfa4-fetch`), which
confines itself with the `fetch` profile of `cfa4-sandbox` before it touches the network.

- `Request` / `FileSpec`: the target directory and the files, each pinned to an `https://` address,
  its size and SHA-256; `Request::validate` is the check both sides make (absolute directory, plain
  file names, HTTPS only).
- `Event`: the JSON lines `cfa4-fetch` writes (`progress`, `done`, `error`).
- `locate()` finds the program (`CFA4_FETCH`, else next to the running program); `run()` starts it,
  sends the request, reports progress and stops it when the caller cancels (removing only the partial files of that download, named by a tag unique to it).

No network code here; the downloading (ureq + rustls) is only in `cfa4-fetch`.
