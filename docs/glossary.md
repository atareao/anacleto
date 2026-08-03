# Anacleto Glossary

## A

**Agent**
: A configurable entity that can be invoked directly by the user. Agents have a description (Markdown file), a model, skills, MCPs, permissions, and optionally a list of subagents. Agents are the top-level invocable unit.

**Anthropic Skill Format**
: A standard for defining skills as Markdown files with YAML frontmatter containing `name` and `description` fields. The frontmatter is used for skill discovery; the body contains instructions loaded on invocation.

## C

**Context Window**
: The maximum number of tokens an LLM can process in a single request. Anacleto uses a configurable percentage (default 50%) of this limit for session history.

**crossterm**
: A cross-platform terminal manipulation library for Rust. Used as the backend for ratatui.

## E

**Engine**
: The core orchestration loop of Anacleto. Manages agent lifecycle, message routing, session state, and coordinates between the TUI, LLM providers, skills, and MCPs.

## H

**History**
: The record of messages exchanged within a session, stored per-session and per-agent in SQLite. Used for context and session resumability.

## L

**LLM Provider**
: A backend service that provides language model inference. Supported providers: Anthropic, OpenAI, Ollama.

## M

**MCP (Model Context Protocol)**
: A JSON-RPC 2.0 based protocol for integrating external tools and services with LLM agents. Anacleto consumes MCP servers over stdio or TCP transport.

**MCP Server**
: An external process or service that implements the MCP spec. Anacleto connects to these but does not manage their lifecycle.

## P

**Permission**
: A rule controlling what an agent/subagent can do. Types: `fs.read`, `fs.write`, `net.http`, `command.run`, `mcp.use`, `env.read`, `skill.use`. Default model: allow by default, deny explicitly.

## R

**ratatui**
: A Rust library for building Terminal User Interfaces. Used as the sole interaction mode for Anacleto.

## S

**Session**
: A complete conversation with Anacleto, persisted in SQLite. Sessions are resumable and contain per-agent message history.

**Skill**
: A specialized capability loaded by an agent, defined as a Markdown file with YAML frontmatter. Skills are independent per agent/subagent and are not inherited.

**Subagent**
: A type of agent that cannot be invoked directly by the user. Subagents are created by their parent agent, receive messages, work independently, and are destroyed after completion. Subagents cannot have their own subagents.

## T

**TUI (Terminal User Interface)**
: The sole user interface of Anacleto, built with ratatui + crossterm. Features chat panel, input panel, agent list, skill/MCP indicators, and subagent tree.

**Tokio**
: The async runtime used by Anacleto. The TUI and engine run as separate Tokio tasks within the same process, communicating via `mpsc` channels.

**Tool Use**
: The LLM's ability to invoke tools (skills or MCPs) during generation. The TUI shows these as visible intermediate steps with live output.