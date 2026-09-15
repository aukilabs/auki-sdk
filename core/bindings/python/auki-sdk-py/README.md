# Auki SDK for Python

Use `auki_sdk` to connect peers, work with Domain data, and run compute or robot
task handlers. Requires Python 3.8+ and Rust 1.89+.

## Build

From the SDK repository root, create a virtual environment and build the binding:

~~~sh
python3 -m venv core/bindings/python/auki-sdk-py/.venv
. core/bindings/python/auki-sdk-py/.venv/bin/activate
python -m pip install 'maturin>=1.5,<2.0'
maturin develop --locked --no-default-features --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml
~~~

This build includes networking, Domain data, and tasks. Omit
`--no-default-features` to include the default application protocols. Custom
Rust protocols must be compiled into the same extension; see
[custom protocols](../../../../docs/how-to/protocols.md).

## Use the binding

| Task | API | Guide or example |
| --- | --- | --- |
| Connect peers | `AukiSession`, `AukiPeer` | [Python Echo](../../../examples/portable-echo/python/README.md) |
| Read and write Domain data | `session.domains()`, `session.data(domain_id)` | [Domain data guide](../../../../docs/how-to/domain-data.md#use-web-or-python) |
| Run compute or robot handlers | `AukiComputeCredential`, `AukiRobotCredential`, `AukiDmsTasks` | [Task guide](../../../../docs/how-to/run-compute-tasks.md) |

`AukiSession.login_dev` signs in to development services. Backend services can
use `login_app_dev`. Both accept a persistent `client_id` for the installation.
Close data clients and peers before closing their session. For tasks, await
`tasks.close()` before closing the machine credential.

The [compute](examples/compute_task.py) and [robot](examples/robot_task.py)
examples include shutdown handling. The [file example](examples/domain_data.py)
streams a 17 MiB file by default, verifies SHA-256, and deletes its unique dev
record. It requires Python 3.9+ and optionally accepts an input file path.

To run the file example, set `AUKI_EMAIL`, `AUKI_PASSWORD`, `AUKI_DOMAIN_ID`, and
`AUKI_CLIENT_ID` for a dev account and Domain approved for data writes:

~~~sh
python core/bindings/python/auki-sdk-py/examples/domain_data.py
~~~

See the [data](../../../../docs/reference/domain-data.md) and
[task](../../../../docs/reference/tasks.md) references for limits, errors,
credential support, and cancellation behavior.

## Test locally

After building, run the data and task suites from the repository root.
These use local DDS, DMS, and data fixtures:

~~~sh
python -m pip install -r core/bindings/python/auki-sdk-py/python_tests/requirements.txt
python -m pytest core/bindings/python/auki-sdk-py/python_tests/test_domain_data.py core/bindings/python/auki-sdk-py/python_tests/test_tasks.py core/bindings/python/auki-sdk-py/python_tests/test_robot_tasks.py -q
~~~
