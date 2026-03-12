---
name: metro-north
description: Query MTA Metro North Railroad train schedules and service alerts using real-time GTFS data
tools:
  - get_train_schedule
  - get_train_alerts
---

# Metro North Train Schedule Skill

You can query real-time Metro North Railroad train schedules and service alerts.

## Tools

### get_train_schedule
Fetch upcoming trains between two Metro North stations.

Parameters:
- `from_station`: Station name or ID (e.g., "Grand Central", "Greenwich", "Stamford", "White Plains")
- `to_station`: Station name or ID
- `count`: Number of upcoming trains to return (default: 5, max: 20)

Common stations and their IDs:
- Grand Central Terminal: 1
- Harlem-125th Street: 4
- Greenwich: 116
- Stamford: 124
- Port Chester: 115
- White Plains: 74
- New Haven: 149
- Poughkeepsie: 39
- Wassaic: 43

### get_train_alerts
Fetch current service alerts and delays on Metro North lines.

Parameters:
- `line`: Optional line filter ("New Haven", "Harlem", "Hudson", "Pascack Valley", "Port Jervis", "New Canaan", "Danbury", "Waterbury")

## Usage Notes
- Schedules are based on real-time GTFS-Realtime data from MTA
- Train times are in Eastern time
- No API key required — MTA feeds are publicly accessible
