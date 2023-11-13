# syntax_surfer
Embeddable Retrieval Engine for On-Device RAG Applications

## Notes on pgvector

While pg-embed, retrieves the binary for your specific machine, it will not have native access to the 'pgvector' extension.
In order to add this extension, you need to install pgvector, based on the following instructions [here](https://github.com/pgvector/pgvector#installation-notes).

It is necessary that you then move three files.

- vector.so: pgvector install path -> ~/.cache/pg-embed/.../lib/postgresql/vector.so
- sql/vector-{VERSION_NUM}.sql -> ~/.cache/pg-embed/../share/postgresql/extension/vector-{VERSION_NUM}.sql
- sql/vector.sql -> ~/.cache/pg-embed/../share/postgresql/extension/vector.sql

It is important to not that the VERSION_NUM of the vector-{VERSION_NUM}.sql file, matches the version specific in the vector.sql file.
