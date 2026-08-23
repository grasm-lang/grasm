# 41 — Custom SPA Router  *(phase 2, project 12)*

**Tests:** one route table matched by one function on both sides of the wire,
and deep links that are real requests.
**Result:** works.

```
grasm run main.gr < /dev/null         # the comparison the harness makes
echo '/users/7/posts/3' | grasm run main.gr
grasm run main.gr -- serve            # then serve index.html over http
```

## The problem this is about

A single-page app has two routers: one in the browser deciding what to draw when
a link is clicked, and one on the server deciding what to send when somebody
pastes a URL. They have to agree. Normally they are two implementations in two
languages that agree by inspection until they do not — and the symptom is a deep
link that 404s when the app itself navigates to it happily.

Here they are **one matcher**. The browser fetches `bundle.gr` — this program as
one file, written by `grasm build --source` — runs it in wasm, and asks it to
match the path. The server calls the same function.

```
$ echo '/users/7/posts/3' | grasm run main.gr
{"name": "user-post", "params": {"id": "7", "post": "3"}}

# and through wasm, exactly as the page does it:
/users/7/posts/3     {"name": "user-post", "params": {"id": "7", "post": "3"}}
```

## What the table decides

```
/                          home
/about                     about
/users/new                 new-user      ← before /users/:id, which would swallow it
/users/:id                 user
/users/:id/posts/:post     user-post
/files/*path               file          ← wildcard, must be last
```

Order matters and having one table means it is decided once rather than twice.
`/users/new` before `/users/:id` is the classic case: the wrong order hands
`new` over as an id, in both routers, and nobody notices until a user is named
`new`.

A wildcard **must be last**, which is checked rather than assumed — a pattern
with one in the middle matches in a way nobody can predict, so it matches
nothing.

A trailing slash is not a navigational decision anybody makes on purpose, so
`/about/`, `//about` and `/about` are one route. A query string is not part of
the route either.

## Deep links are real requests

The server renders the same view the browser would have drawn, so pasting a URL
and clicking to it produce the same page. That is the thing an SPA usually gets
wrong, so the harness checks it rather than claiming it — for each path, that
the served document *contains* the client's own view.

```
/                    200 <h2>Home</h2>
/users/7             200 <h2>User 7</h2>
/users/7/posts/3     200 <h2>Post 3</h2>
/files/a/b/c.txt     200 <h2>File</h2>
/nowhere             404 <h2>Not found</h2>
```

`/nowhere` is a **404 that still renders the page**. A router that returns a
bare 404 body for an unknown path inside an app is a router that drops the user
out of the app.

## A check that passed for the wrong reason

The escaping test first used `/users/<script>alert(1)</script>`, which does not
match anything — `</script>` contains a slash, so the path becomes three
segments. The check passed because the payload never reached the view, which is
an accident of routing rather than a defence.

It uses `<img src=x onerror=alert(1)>` now, which has no slash, reaches the view
and comes out escaped:

```
id captured: <img src=x onerror=alert(1)>
rendered:    <h2>User &lt;img src=x onerror=alert(1)&gt;</h2>
```

A path segment reaches the page in both routers, from the same line, so without
this it is a reflected cross-site-scripting hole twice over.

## The honest limit: the *view* is still written twice

The matcher is shared. The rendering is not — `main.gr` has `view` and
`index.html` has `viewFor`, and they are the same six cases in two languages.

That is the smaller half of the problem and it is still a copy. Sharing it would
mean the browser asking the Grasm view for HTML on every navigation, which is
one wasm call per click and entirely possible; the reason it is not done here is
that this project is about *routing*, and a page that got its HTML from Grasm
but decided its own routes would be the wrong half shared. The harness checks
the two agree on the server side; nothing checks `viewFor` against `view`, and
that gap is real.

## Verified live

```
$ curl localhost:8982/api/match/users/7/posts/3
{"name": "user-post", "params": {"id": "7", "post": "3"}}
```

The `/api/match/...` route exists so a client that cannot run the matcher can
ask, and the harness checks it gives the same answer as running it directly.
