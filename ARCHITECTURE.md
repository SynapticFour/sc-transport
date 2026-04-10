# Architecture Overview

This document explains the system at a level that helps contributors reason about change impact.

## Goals

- Keep behavior predictable and testable.
- Keep security and operational concerns explicit.
- Keep extension points clear for new features.

## High-level structure

Describe the major modules/services and their responsibilities here.

## Data and control flows

Describe how requests/events flow through the system and where validation, authorization, and persistence happen.

## Reliability and security boundaries

Describe trust boundaries, secret handling, and failure modes that contributors should keep in mind.

## Key extension points

List where new integrations, endpoints, or jobs should be added.
