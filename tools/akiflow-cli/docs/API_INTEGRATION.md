# Akiflow CLI - API Integration Guide

## Overview
This document describes how to integrate with the Akiflow API using the AkiflowClient.

## Authentication
The CLI uses browser-based token extraction to authenticate with Akiflow:
- Supports Chrome, Arc, Brave, Edge, and Safari
- Automatically scans browser cookies for JWT tokens
- Stores credentials securely in Keychain (macOS) or XDG (Linux)

## API Methods

### getTasks()
Retrieves all tasks from Akiflow.

### upsertTasks(tasks)
Creates or updates tasks in Akiflow.

### getLabels()
Retrieves all labels.

### getTags()
Retrieves all tags.

### getTimeSlots()
Retrieves all time slots.
