---
name: metro-north
version: 1.0.0
description: Real-time MTA Metro North Railroad train schedules and service alerts (no API key required)
tools:
  - name: get_train_schedule
    description: Fetch upcoming trains between two Metro North stations
    parameters:
      from_station:
        type: string
        description: Origin station name (partial match supported)
        example: "Grand Central Terminal"
      to_station:
        type: string
        description: Destination station name (partial match supported)
        example: "Stamford"
      count:
        type: integer
        description: Number of upcoming trains to return
        default: 5
  - name: get_train_alerts
    description: Fetch active Metro North service alerts and delays
    parameters:
      line:
        type: string
        description: Optional line to filter alerts by
        enum: ["New Haven", "Harlem", "Hudson", "Pascack Valley", "Port Jervis", "New Canaan", "Danbury", "Waterbury"]
examples:
  - "When's the next train from Grand Central to Stamford?"
  - "Show me 3 trains to Greenwich"
  - "Any delays on the New Haven line today?"
  - "What trains go from White Plains to Grand Central in the next hour?"
  - "Is the Hudson line running on time?"
  - "Next train to New Haven?"
---

## Key Stations

Grand Central Terminal, Harlem-125th Street, Stamford, Greenwich, Port Chester,
White Plains, New Haven, Poughkeepsie, Wassaic, Mount Kisco, Scarsdale, Yonkers,
Tarrytown, Ossining, Croton-Harmon, Peekskill, Beacon, Dobbs Ferry, Irvington.

Schedules are real-time from the MTA GTFS-Realtime feed. Times are Eastern.
