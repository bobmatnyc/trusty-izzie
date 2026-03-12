---
name: weather
version: 1.0.0
description: Real-time weather forecasts (Open-Meteo) and NWS severe weather alerts. No API key required.
tools:
  - name: get_weather
    description: Get weather forecast for any location
    parameters:
      location:
        type: string
        description: City name or address (defaults to Hastings-on-Hudson, NY)
        example: "New York City"
      days:
        type: integer
        description: Forecast days (1-7)
        default: 3
  - name: get_weather_alerts
    description: Get active NWS severe weather alerts (US only)
    parameters:
      location:
        type: string
        description: Location to check alerts for
        example: "Hastings-on-Hudson"
examples:
  - "What's the weather today?"
  - "Will it rain this weekend?"
  - "Weather forecast for the next 3 days"
  - "Any storm warnings near me?"
  - "What's the weather in Boston tomorrow?"
  - "Is it going to snow this week?"
  - "Should I bring an umbrella today?"
---

## Proactive Weather Alerts

Izzie proactively notifies you at 7:30 AM when:
- Active NWS Severe or Extreme weather alert in your area
- Heavy rain forecast (>=80% chance, >=0.5" expected)
- Thunderstorm or severe weather code in next 48 hours
- Extreme heat (>=95F) or dangerous cold (<=15F overnight)
- High winds (>=35mph)

Data sources: Open-Meteo (forecast) + National Weather Service (alerts).
No API key required.
