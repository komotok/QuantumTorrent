# Search plugins

A plugin is a JSON file describing **where to send a query** and **how to read
the response**. Drop one into the app's `search-plugins` folder (Search →
Sources → Open folder) and reopen the dialog.

Plugins are declarative on purpose. qBittorrent's plugins are Python scripts,
which means installing one is arbitrary code execution. Here the worst a bad
plugin can do is point at a URL and mis-parse the reply.

## Fields

| Field  | Required | Meaning |
|--------|----------|---------|
| `id`   | yes | Unique. Reusing a built-in's id replaces it. |
| `name` | yes | Shown in the source column and the Sources list. |
| `url`  | yes | Request URL. `{query}` is replaced with the percent-encoded search terms. |
| `browseUrl` | no | Listing shown before anything is typed. No `{query}`. A source without one contributes nothing to the browse view. |
| `kind` | yes | `json` or `rss`. |
| `json` | for `kind: json` | Field mapping, see below. |
| `site` | no | Homepage, shown in the Sources list. |

### `json` mapping

| Field          | Required | Meaning |
|----------------|----------|---------|
| `results`      | yes | Slash path to the results array, e.g. `response/docs`. |
| `name`         | yes | Field holding the title. |
| `link`         | yes | Field holding a magnet/`.torrent` URL, or an id to feed `linkTemplate`. |
| `linkTemplate` | no  | Template with `{value}`, for APIs returning an id rather than a URL. |
| `size`         | no  | Field holding size in bytes. |
| `seeders`      | no  | Field holding the seeder count. Used for sorting. |

Sizes and seeder counts are accepted as either numbers or numeric strings.

### `rss`

No mapping needed. Each `<item>` (or Atom `<entry>`) becomes a result:

- **name** — `<title>`
- **link** — `<enclosure url>` if present, else `<link>`. Enclosure is preferred
  because `<link>` usually points at a details page rather than the torrent.
- **size** — `<enclosure length>`, else a `<size>` element
- **seeders** — any `seeders` element, namespaced or not

## Example

```json
{
  "id": "example-json",
  "name": "Example Tracker",
  "site": "https://example.org",
  "kind": "json",
  "url": "https://example.org/api/search?q={query}&limit=50",
  "json": {
    "results": "data/items",
    "name": "title",
    "size": "length_bytes",
    "seeders": "seeds",
    "link": "info_hash",
    "linkTemplate": "magnet:?xt=urn:btih:{value}"
  }
}
```

```json
{
  "id": "example-rss",
  "name": "Example Feed",
  "kind": "rss",
  "url": "https://example.org/rss?q={query}"
}
```

## Notes

- Results link into the normal add flow, so you still get metadata preview and
  per-file selection before anything downloads.
- A source that errors is reported in the dialog rather than silently dropped —
  "no results" and "that source is broken" are different problems.
- Each source is capped at 60 results and a 20 second timeout.
- Only sources you add yourself are queried, plus the built-ins. Nothing is
  contacted until you press Search.
