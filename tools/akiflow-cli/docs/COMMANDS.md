# Akiflow CLI Commands Reference

## Task Management

### af ls
List all tasks with optional filters.
```bash
af ls --label "work" --status "active"
```

### af add
Add a new task with natural language date parsing.
```bash
af add "Buy groceries" --due "tomorrow" --duration "1h"
```

### af do
Mark a task as complete.
```bash
af do <task-id>
```

### af task
Manage task properties.
```bash
af task edit <task-id> --title "New title"
af task move <task-id> --project "Work"
af task plan <task-id> --date "2026-02-15"
af task snooze <task-id> --duration "2h"
af task delete <task-id>
```

## Project Management

### af project
Manage projects.
```bash
af project ls
af project create "New Project"
af project delete "Project Name"
```

## Calendar & Time Blocking

### af cal
View calendar with tasks.
```bash
af cal --month "2026-02"
```

### af block
Create time blocks.
```bash
af block "Focus Time" --start "10:00" --duration "2h"
```

## Authentication

### af auth
Authenticate with Akiflow.
```bash
af auth
```

## Shell Completion

### af completion
Generate shell completion scripts.
```bash
af completion bash > ~/.bashrc
af completion zsh > ~/.zshrc
af completion fish > ~/.config/fish/completions/af.fish
```
