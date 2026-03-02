# Contributing to Conclave

Thank you for considering contributing to Conclave. This document outlines the guidelines for contributing to ensure a smooth and effective development process. Should you have any questions or require assistance, please do not hesitate to reach out to the project maintainers.

## Commit Messages

Commit messages must follow the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) specification. This helps maintain a consistent and informative project history.

### Commit Message Format

```text
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

#### Common Types

- **feat**: A new feature
- **fix**: A bug fix
- **docs**: Documentation updates
- **perf**: Performance improvements that do not affect the code's behavior
- **style**: Changes that do not affect the code's functionality (e.g., formatting)
- **refactor**: Code changes that neither fix a bug nor add a feature
- **test**: Adding or modifying tests
- **chore**: Maintenance or other non-functional updates

#### Common Scopes

Including a scope is optional but is strongly encouraged. One commit should only address changes to a single module or component. If a change must affect multiple modules, use `*` as the scope.

- **server**: The Conclave server (`conclave-server`)
- **client**: The client library (`conclave-client`)
- **cli**: The TUI client (`conclave-cli`)
- **gui**: The GUI client (`conclave-gui`)
- **proto**: The protobuf definitions (`conclave-proto`)
- **spec**: The protocol specification (`docs/spec`)

#### Example

```
feat(server): add configurable session token lifetime

Add the `token_ttl_seconds` field to the server config to allow
operators to control session expiration.

Closes #123
```

## Coding Standards

All code contributions must strictly follow the coding standards outlined in this section and in `AGENTS.md`. Before submitting any code changes, ensure your code adheres to these guidelines.

### Rust Code Style

- Follow the conventions in `AGENTS.md`.
- Run `cargo fmt` before submitting.
- Run `cargo clippy --workspace` and resolve all warnings.
- Run `cargo test --workspace` and ensure all tests pass.

### Code Formatting

All Rust code must be formatted using `cargo fmt` before submitting a pull request.

## Submitting a Pull Request

1. **Fork the repository**: Create a personal fork of the project.
2. **Create a branch**: Create a new branch for your changes:
   ```bash
   git checkout -b <type>/<scope>
   ```
3. **Write code**: Make your changes, ensuring they adhere to the coding standards and are properly documented.
4. **Commit changes**: Write clear and descriptive commit messages using the Conventional Commits format.
5. **Push changes**: Push your branch to your fork:
   ```bash
   git push origin <type>/<scope>
   ```
6. **Open a pull request**: Submit your pull request to the `master` branch of the original repository. Include a clear description of the changes made and reference any relevant issues.

## Code Reviews

All pull requests will undergo a code review. Please expect feedback from the maintainers after you submit the pull request. We may need further information or changes before merging your pull request.

## AI Use Policy

The use of AI tools (e.g., LLMs, code assistants) for writing code, documentation, or other contributions is permitted under the following conditions:

1. **You must fully understand the changes.** Do not submit AI-generated code you cannot explain or defend during review.
2. **You must proofread and test all AI output.** AI-generated code must be reviewed for correctness, style compliance, and tested before submission.
3. **You must declare AI use.** If AI tools were used to generate or substantially edit your contribution, state so in the pull request description.
