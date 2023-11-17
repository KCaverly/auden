# Yet Another Retrieval Server
Embeddable Retrieval Engine for On-Device Retrieval Applications

This is very much a work in progress project, in which I am hoping to create a standalone directory based retrieval engine in Rust. The hope is that this can either be leveraged as a package directly inside another rust application to manage on-device retrieval, or run as a binary and exposed over something like gRPC.

This project is primarily an experiment, right now, and very simple.

At a high level it uses:
- tokio: Fast async runtime
- postgres: Leverages an embedded postgresql database for storage
- pgvector: The pg extension for fast in database vector search

## Notes on pgvector

While pg-embed, retrieves the binary for your specific machine, it will not have native access to the 'pgvector' extension.
In order to add this extension, you need to install pgvector, based on the following instructions [here](https://github.com/pgvector/pgvector#installation-notes).

It is necessary that you then move three files.

- vector.so: pgvector install path -> ~/.cache/pg-embed/.../lib/postgresql/vector.so
- sql/vector-{VERSION_NUM}.sql -> ~/.cache/pg-embed/../share/postgresql/extension/vector-{VERSION_NUM}.sql
- sql/vector.sql -> ~/.cache/pg-embed/../share/postgresql/extension/vector.sql

It is important to not that the VERSION_NUM of the vector-{VERSION_NUM}.sql file, matches the version specific in the vector.sql file.

Unfortunately this is a hack, and I think a longer term goal of the project is either, to figure out how to ship pgvector with the embedded postgresql binary, or move to something else entirely.
