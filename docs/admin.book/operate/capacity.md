---
id: 01M63CFE62RWV0Q910TAABPK62
title: Capacity and scaling
vellum: 1
---

# Capacity and scaling

## Sizing {#sizing}

| People | Profile | Suggested resources |
| --- | --- | --- |
| 1 to 20 | single | 1 vCPU, 512 MB, SSD |
| 20 to 300 | team | 4 vCPU, 8 GB for Vellum and Postgres together |
| 300 and more | org | 2+ replicas of each service; Postgres sized to the index |

The team profile is tested at 50 people editing at once and 2,000 page
reads per second.

## Scaling out {#scale-out}

Every service in the binary can run on its own: gateway, identity,
pages, content, authorization, collaboration, render, export,
collections, search, notifications and audit. In Helm, set the replica
count per service. Collaboration rooms and search shards are assigned by
space, so a large space never shares a node's capacity with every other.

## Caches {#caches}

Pages are rendered when they are saved and served from a cache with
ETags, so reads cost almost nothing. Put a CDN in front of the public
site; private pages are cached per permission set.
