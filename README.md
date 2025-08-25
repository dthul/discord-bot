# Add bot to Discord Server

Use the following URL:

prod: `https://discordapp.com/api/oauth2/authorize?client_id=600752105518792716&permissions=834873616&scope=bot`
test: `https://discordapp.com/api/oauth2/authorize?client_id=643523617702936596&permissions=834873616&scope=bot`

## Permissions

The bot needs the following permissions:
* Manage Roles
* Manage Channels
* Manage Webhooks
* Read Message History
* Send Messages
* Manage Messages
* Mention @everyone
* Mute Members
* Move Members
* Deafen Members
* Priority Speaker

Permission Calculator: https://discordapi.com/permissions.html#834873616

# Authorize Slash Commands

prod: `https://discordapp.com/api/oauth2/authorize?client_id=600752105518792716&scope=applications.commands`
test: `https://discordapp.com/api/oauth2/authorize?client_id=643523617702936596&scope=applications.commands`

# Build and run using Docker

See `docker.md`.

## Database connection

Depending on the settings a database connection might be required to build. An SSH tunnel to the database can be established as follows:

`$ ssh -N -L 5432:localhost:5432 <username>@bot.swissrpg.ch`

# Redis

Configuration file can be found in `/etc/redis/redis.conf`. By default backups will be found at `/var/lib/redis/dump.rdb` and `/var/lib/redis/appendonly.aof`.
Important settings:

```
# General settings
bind 127.0.0.1 ::1
protected-mode yes

# RDB settings
save 900 1
save 300 10
save 60 10000
stop-writes-on-bgsave-error yes
rdbchecksum yes

# AOF setings
appendonly yes
appendfsync everysec
aof-use-rdb-preamble yes
```

# Postgres

Install according to the instructions on the website. Under Ubuntu, a new `postgres` user will be created and the database cluster will be initialized in `/var/lib/postgresql/13/main` (or appropriate other major version number).

Create Postgres database users ("roles" in Postgres speak) for the Unix users which need a database (`bot` and `bottest` probably):

`$ sudo -u postgres createuser --interactive` (x2)
```
Enter name of role to add: bot
Shall the new role be a superuser? (y/n) n
Shall the new role be allowed to create databases? (y/n) n
Shall the new role be allowed to create more new roles? (y/n) n
```

Create databases for the `bot` and `bottest` users:

`$ sudo -u postgres createdb bot --owner bot`
`$ sudo -u postgres createdb bottest --owner bottest`

Revoke public connection rights to the newly created databases:

`$ sudo -u postgres psql`
`# REVOKE connect ON DATABASE bot FROM PUBLIC;`
`# REVOKE connect ON DATABASE bottest FROM PUBLIC;`

## Optional: create a superuser with password login for database managing tools

`$ sudo -u postgres createuser --interactive`
```
Enter name of role to add: daniel
Shall the new role be a superuser? (y/n) y
```

Set a password

`$ sudo -u postgres psql`
`# ALTER USER daniel ENCRYPTED PASSWORD '...'`

## Optional: create a read-only user for sqlx

`$ sudo -u postgres psql bottest`
`# CREATE USER sqlx WITH PASSWORD '...';`
`# GRANT CONNECT ON DATABASE bottest TO sqlx;`
`# GRANT USAGE ON SCHEMA public TO sqlx;` (requires to be connected to the right database)

# Metabase

`$ sudo useradd --system --create-home --shell /bin/bash metabase`
`$ sudo apt-get install openjdk-11-jre` (or later version)
`$ sudo -i -u metabase`
`$ wget https://downloads.metabase.com/v0.37.8/metabase.jar` (or later version)

## Optional: create a read-only Postgres user for metabase

`$ sudo -u postgres psql bottest`
`# CREATE USER metabase WITH PASSWORD '...';`
`# GRANT CONNECT ON DATABASE bottest TO metabase;`
`# GRANT USAGE ON SCHEMA public TO metabase;` (requires to be connected to the right database)
`# GRANT SELECT ON ALL TABLES IN SCHEMA public TO metabase;` (requires to be connected to the right database)
`# ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO metabase;` (requires to be connected to the right database)

# Backups to Amazon S3

Copy/symlink `swissrpg-backup.service` and `swissrpg-backup.timer` to `/etc/systemd/system/`:

`$ scp swissrpg-backup.* root@167.235.157.111:/etc/systemd/system/`

Start the timer:

`$ systemctl start swissrpg-backup.timer`

and to enable it permanently:

`$ systemctl enable swissrpg-backup.timer`

NOTE: call `systemctl daemon-reload` after modifying service files

To check the scheduled timers, use:

`$ systemctl list-timers`