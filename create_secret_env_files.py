#!/usr/bin/env python3

import csv

with open("secrets.csv", newline="") as f:
    reader = csv.reader(f, delimiter="=")
    secrets = {
        row[0]: row[1]
        for row in reader
        if len(row) == 2 and len(row[0]) > 0 and row[0][0] != "#"
    }

# Secrets for PostgreSQL
with open("secrets-postgres.sh", "w") as f:
    f.write(f"POSTGRES_PASSWORD={secrets['POSTGRES_PASSWORD_POSTGRES']}\n")

# Secrets for production bot
with open("secrets-prod.sh", "w") as f:
    f.write(f"DISCORD_TOKEN={secrets['DISCORD_TOKEN_PROD']}\n")
    f.write(f"DISCORD_APPLICATION_ID={secrets['DISCORD_APPLICATION_ID_PROD']}\n")
    f.write(f"SWISSRPG_API_URL={secrets['SWISSRPG_API_URL_PROD']}\n")
    f.write(f"SWISSRPG_API_SECRET={secrets['SWISSRPG_API_SECRET_PROD']}\n")
    f.write(f"MEETUP_CLIENT_ID={secrets['MEETUP_CLIENT_ID_PROD']}\n")
    f.write(f"MEETUP_CLIENT_SECRET={secrets['MEETUP_CLIENT_SECRET_PROD']}\n")
    f.write(f"STRIPE_CLIENT_SECRET={secrets['STRIPE_CLIENT_SECRET_PROD']}\n")
    f.write(
        f"STRIPE_WEBHOOK_SIGNING_SECRET={secrets['STRIPE_WEBHOOK_SIGNING_SECRET_PROD']}\n"
    )
    f.write(f"API_KEYS={secrets['API_KEYS_PROD']}\n")
    f.write(
        f"DATABASE_URL=postgres://{secrets['POSTGRES_USER_PROD']}:{secrets['POSTGRES_PASSWORD_PROD']}@{secrets['POSTGRES_HOST']}/{secrets['POSTGRES_DATABASE_PROD']}"
    )

# Secrets for test bot
with open("secrets-test.sh", "w") as f:
    f.write(f"DISCORD_TOKEN={secrets['DISCORD_TOKEN_TEST']}\n")
    f.write(f"DISCORD_APPLICATION_ID={secrets['DISCORD_APPLICATION_ID_TEST']}\n")
    f.write(f"SWISSRPG_API_URL={secrets['SWISSRPG_API_URL_TEST']}\n")
    f.write(f"SWISSRPG_API_SECRET={secrets['SWISSRPG_API_SECRET_TEST']}\n")
    f.write(f"MEETUP_CLIENT_ID={secrets['MEETUP_CLIENT_ID_TEST']}\n")
    f.write(f"MEETUP_CLIENT_SECRET={secrets['MEETUP_CLIENT_SECRET_TEST']}\n")
    f.write(f"STRIPE_CLIENT_SECRET={secrets['STRIPE_CLIENT_SECRET_TEST']}\n")
    f.write(
        f"STRIPE_WEBHOOK_SIGNING_SECRET={secrets['STRIPE_WEBHOOK_SIGNING_SECRET_TEST']}\n"
    )
    f.write(f"API_KEYS={secrets['API_KEYS_TEST']}\n")
    f.write(
        f"DATABASE_URL=postgres://{secrets['POSTGRES_USER_TEST']}:{secrets['POSTGRES_PASSWORD_TEST']}@{secrets['POSTGRES_HOST']}/{secrets['POSTGRES_DATABASE_TEST']}"
    )

with open("secrets-backup.sh", "w") as f:
    f.write("S3_BUCKET_ID=qd2shp96ohk3mfyq\n")
    f.write(f"PSQL_BOT_PW={secrets['POSTGRES_PASSWORD_PROD']}\n")
    f.write(f"PSQL_BOTTEST_PW={secrets['POSTGRES_PASSWORD_TEST']}\n")
