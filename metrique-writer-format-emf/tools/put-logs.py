#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["boto3"]
# ///

import boto3
import argparse
import json
import time

def main():
    parser = argparse.ArgumentParser(
        description='''\
Helper test program to be able to validate EMF processing.

This program is intended for use while developing `metrique-writer-format-emf` - when
a new kind of EMF record is created, this program can be used to validate that
it behaves in CloudWatch as the developer expect.

In particular, this program is NOT intended for use by *users* of `metrique-writer-format-emf`.

Just uploads a log to CloudWatch logs. You can then go to CloudWatch Metrics to see
that the metrics have been uploaded correctly. Note that it might take
several minutes for metrics to be processed.

To use this script, you'll first need to create a log group and a log stream, for example

`aws logs create-log-group --log-group-name TestLogGroup &&
    aws logs create-log-stream --log-group-name TestLogGroup --log-stream-name TestLogStream`.

Then call this script with the log group, and a log file
```
./put-logs.py ./file.json --log-group=TestLogGroup --log-stream=TestLogStream
```

The expected format of `file.json` is either a single JSON dictionary
```
{"_aws":{"CloudWatchMetrics":[{"Namespace":"MyNS","Dimensions":[["label"]],"Metrics":[{"Name":"my_counter"}]}],"Timestamp":1},"label":"value1","my_counter":1}
```

Or a JSON array of dictionaries
```
[
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"MyNS","Dimensions":[["label"]],"Metrics":[{"Name":"my_counter"}]}],"Timestamp":1},"label":"value2","my_counter":2},
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"MyNS","Dimensions":[[]],"Metrics":[{"Name":"my_counter"}]}],"Timestamp":1},"my_counter":3}
]
```

NOTE: this will automatically "fix" the timestamps in the metrics to be the current
time. This is very good for testing because the timestamps from the unit tests have
"canned" times which are very far from the present and will not work in CloudWatch
Metrics. It is of course not the right thing for any sort of production use.
'''
        )
    parser.add_argument('file', help='''
Path to the log file to read, should be valid JSON and contain a log entry (JSON dictionary)
or an array of them (JSON array of dictionaries)''')
    parser.add_argument('--log-group', help = 'log group to use', required=True)
    parser.add_argument('--log-stream', help = 'log stream to use', required=True)
    parser.add_argument('--region', help = 'region to use')

    args = parser.parse_args()

    # Read the file contents into the log variable
    logs = []
    with open(args.file, 'r') as file:
        log = file.read()
    levts = json.loads(log)
    if isinstance(levts, dict):
        levts = [levts]
    for e in levts:
        # Put a timestamp that is slightly before now for the metrics, to ensure they are counted correctly
        #
        # Since this is normally use
        e.get('_aws', {})['Timestamp'] = int(time.time() * 1000 - 1000)

    kwargs = {}
    if args.region is not None:
        kwargs['region_name'] = args.region
    session = boto3.session.Session(**kwargs)
    logs = session.client('logs')
    logs.put_log_events(
        logGroupName=args.log_group,
        logStreamName=args.log_stream,
        logEvents=[
            { 'message': json.dumps(m), 'timestamp': int(time.time() * 1000) }
            for m in levts
        ]
    )

if __name__ == '__main__':
    main()
