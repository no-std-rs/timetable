#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo rustc -p timetable-android --lib --release --locked --features host-test --crate-type cdylib
mkdir -p target/android-jni-smoke
javac -d target/android-jni-smoke android/java/org/nostdrs/timetable/Native.java android/tests/NativeSmoke.java
java -Djava.library.path=target/release -cp target/android-jni-smoke NativeSmoke tablet/tests/fixtures/settings.json gtfs/tests/fixtures/gtfs.zip
javac -cp target/android-jni-smoke -d target/android-jni-smoke android/tests/JourneySmoke.java
java -Djava.library.path=target/release -cp target/android-jni-smoke JourneySmoke tablet/tests/fixtures/journey.json display/tests/fixtures/journey.zip

javac -d target/android-jni-smoke android/java/org/nostdrs/timetable/FeedCache.java android/tests/FeedCacheSmoke.java
java -cp target/android-jni-smoke org.nostdrs.timetable.FeedCacheSmoke
