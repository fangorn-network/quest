"""Mississippi's places from an Overture Maps release, to a local parquet file (run by fetch.sh).

    .venv/bin/python pipeline/fetch_overture.py <release> <out.parquet>

DuckDB reads only the row groups inside the state's bounding box from Overture's public S3 bucket. Kept: places
whose address says Mississippi, with every column the build uses. Filtering by confidence and status happens in
the build, so it can change without a new download. Overture's places are CDLA-Permissive-2.0.
"""
import sys

import duckdb

release, out = sys.argv[1], sys.argv[2]
con = duckdb.connect()
con.execute("INSTALL httpfs; LOAD httpfs; SET s3_region='us-west-2';")
con.execute(f"""
COPY (
  SELECT id, names."primary" AS name, basic_category, taxonomy."primary" AS category, confidence, operating_status,
         addresses[1].freeform AS street, addresses[1].locality AS town, addresses[1].postcode AS postcode,
         bbox.xmin AS lon, bbox.ymin AS lat,
         list_distinct(list_transform(sources, lambda s: s.license)) AS licenses
  FROM read_parquet('s3://overturemaps-us-west-2/release/{release}/theme=places/type=place/*', hive_partitioning=1)
  WHERE bbox.xmin BETWEEN -91.70 AND -88.05 AND bbox.ymin BETWEEN 30.10 AND 35.05
    AND addresses[1].region IN ('MS', 'Mississippi')
) TO '{out}' (FORMAT parquet)""")
n, licenses = con.execute(f"SELECT count(*), list_distinct(flatten(list(licenses))) FROM read_parquet('{out}')").fetchone()
print(f"overture {release}: {n} Mississippi places, licenses {licenses}")
