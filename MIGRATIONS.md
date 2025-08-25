We use sqlx-cli to handle migrations.

# Install sqlx CLI

```bash
cargo install sqlx-cli --no-default-features --features native-tls,postgres
```

# Update cached queries

```bash
cargo sqlx prepare --workspace -- --all-targets --all-features
```

# Migrations

## Create a new migration

```bash
sqlx migrate add -r <name>
```

Now edit the newly created `*.up.sql` and `*.down.sql` files in the [migration/](./migrations/) folder.

## Run the migrations

Make sure the database connection information in [.env](./.env) is correct (use the `bottest` or `bot` user respectively), then do:

```bash
sqlx migrate run
```

<!-- We use [Tusker](https://github.com/bikeshedder/tusker) to handle database migrations.

# How to modify the schema

The schema is defined in [schema.sql](./schema.sql). Modify this file to reflect the state you would like to have.

Once done, run Tusker to create the necessary migration:

```bash
uv pip install psycopg2-binary~=2.9.5 importlib-metadata~=1.0 migra~=3.0.1621480950 tomlkit~=0.11 sqlalchemy~=1.4.25 setuptools
uv pip install tusker --no-deps
```

Make sure the database connection is correctly configured in (tusker.toml)[./tusker.toml], then create a diff:

python -c 'import tusker; tusker.main()' diff -->
