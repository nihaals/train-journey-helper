# train-journey-helper

A simple train tracking tool for a recurring UK train trip with API endpoints for setting the current state and sends notifications through Home Assistant. Currently made specifically for my journey type of one change and there being a preferred station to use on the return on the same line as the "home" station.

## How it works

The scheduler polls once a minute. On the configured travel day it:

1. Finds outbound services that can reach the final destination by the target time, allowing for walks at either end and between interchange stations
2. Sends an initial report containing outbound and estimated return options
3. Updates its recommendations as the journey state is changed through the HTTP API
4. Monitors return options after the user is marked as being at the destination
5. Stops polling when the journey is complete and waits for the next configured travel day

All journey times are interpreted in the `Europe/London` time zone.

## Requirements

- A [Realtime Trains API token](https://api-portal.rtt.io/)
- A Home Assistant long-lived access token and notification service

## Configuration

Create a JSON file with the following shape (the station values are three-letter CRS codes):

```json
{
  "stations": {
    "home": "AAA",
    "line_one_interchange_primary": "BBB",
    "line_one_interchange_return_preferred": "CCC",
    "destination_line_interchange": "DDD",
    "destination": "EEE"
  },
  "walk": {
    "home_to_station_1_minutes": 10,
    "station_2_to_4_minutes": 12,
    "station_4_to_3_minutes": 8,
    "station_5_to_final_destination_minutes": 15
  },
  "destination_arrival_time": "09:30",
  "destination_stay_estimate_minutes": 480,
  "travel_day": "monday",
  "listen_addr": "127.0.0.1:3000",
  "healthcheck_url": "https://example.com/healthcheck",
  "home_assistant": {
    "base_url": "https://home-assistant.example.com/api/",
    "token": "HOME_ASSISTANT_LONG_LIVED_ACCESS_TOKEN",
    "notify_service": "mobile_app_phone"
  },
  "rtt": {
    "token": "REALTIME_TRAINS_API_TOKEN"
  }
}
```

- The two interchange fields allow the preferred station for the return journey to differ from the primary outbound interchange
- Walking durations must be positive whole minutes
- `healthcheck_url` may be `null` to disable calling a healthcheck endpoint every minute

You can also use the [`pkl/Config.pkl`](pkl/Config.pkl) [Pkl](https://pkl-lang.org/) template.

Validate and print a configuration with:

```sh
train-journey-helper config --config config.json
```

## Running

Build and run the scheduler and web server:

```sh
train-journey-helper run --config config.json
```

Every command is documented in `--help`.

## HTTP API

The server listens on `listen_addr`. Authentication is not implemented.

### Configuration and get service list endpoints

| Method | Path | Description |
| --- | --- | --- |
| `GET` | `/config` | Return the configured station mapping (credentials are not included) |
| `POST` | `/services/leg/home-to-primary-interchange` | List services for the first outbound leg |
| `POST` | `/services/leg/interchange-to-destination` | List services for the final outbound leg |
| `POST` | `/services/leg/destination-to-interchange` | List services for the first return leg |
| `POST` | `/services/leg/return-preferred-interchange-to-home` | List services from the preferred return interchange |
| `POST` | `/services/leg/primary-interchange-to-home` | List return services from the primary interchange |

Service list requests take a London local date and time and return matching trains ordered by departure:

```sh
curl -X POST http://127.0.0.1:3000/services/leg/home-to-primary-interchange \
  -H 'content-type: application/json' \
  -d '{"time":"1970-01-01T08:00:00"}'
```

Each result includes its `service_id`, stations, operator, route destination, scheduled and estimated departure/arrival timestamps, and departure platform.

### Update journey state endpoints

| Method | Path | Description |
| --- | --- | --- |
| `POST` | `/status/on-train/home-to-primary-interchange` | Select a service for the first outbound leg |
| `POST` | `/status/on-train/interchange-to-destination` | Select a service for the final outbound leg |
| `POST` | `/status/on-train/destination-to-interchange` | Select a service for the first return leg |
| `POST` | `/status/on-train/return-preferred-interchange-to-home` | Select a service for the preferred final return leg |
| `POST` | `/status/on-train/primary-interchange-to-home` | Select a service for the primary final return leg |
| `POST` | `/status/at-destination` | Begin monitoring the return journey |
| `POST` | `/status/complete` | Complete this journey and wait for the next one |
| `POST` | `/status/skip-day` | Skip the current journey and wait for the next one |
| `POST` | `/notification/resend` | Resend the most recent notification; returns `404` if none exists |

The `on-train` endpoints take a service ID returned by a service list endpoint:

```sh
curl -X POST http://127.0.0.1:3000/status/on-train/home-to-primary-interchange \
  -H 'content-type: application/json' \
  -d '{"service_id":"SERVICE_ID"}'
```

Successful state changes return `204 No Content`.
