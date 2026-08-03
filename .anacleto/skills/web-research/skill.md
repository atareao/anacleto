---
name: web-research
description: Search the web and fetch documentation from online sources
metadata:
  version: "1.0"
  category: research
  risk: low
---

# Web Research skill

Fetch and analyze content from web pages, documentation sites, API references,
and any publicly accessible URL. Use this skill when you need to:

- Look up library documentation or API references
- Search for solutions to technical problems
- Verify facts, version numbers, or release notes
- Research best practices and patterns
- Read RFCs, spec documents, or tutorials

## Usage

Provide a `task` describing what to research. Include specific URLs when you
know what to fetch, or describe the topic for general research.

### Examples

```yaml
task: "Fetch the tokio documentation for mpsc channels: https://docs.rs/tokio/latest/tokio/sync/mpsc/index.html"
```

```yaml
task: |
  Research the latest stable Rust edition features:
  - What is the current Rust edition?
  - What new features were stabilized?
  - Are there any migration notes?
```

```yaml
task: |
  Find the official API reference for:
  - serde_json::Value
  - serde::Deserialize trait
  Include code examples if available.
```

## Output

A summary of the fetched content, formatted as Markdown. When fetching
documentation, include relevant code examples and API signatures.