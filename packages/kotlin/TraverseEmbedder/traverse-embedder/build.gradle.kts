plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
    id("com.vanniktech.maven.publish")
}

android {
    namespace = "dev.traverse.embedder"
    compileSdk = 35

    defaultConfig {
        minSdk = 28
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // #1611: each instrumented test runs in its own process (orchestrator), so one test can
        // make the native library fail to load without affecting the others.
        testInstrumentationRunnerArguments["clearPackageData"] = "true"
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }

    // Decision 108: the arm64-v8a / x86_64 traverse-android-host libraries
    // that scripts/build_android_host_ndk.sh cross-builds land in
    // build/jniLibs and ship inside the AAR (nothing binary is committed).
    sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("jniLibs"))

    // #1611: instrumented tests read the signed fixtures and the rights suite
    // from the test APK's assets (repository `fixtures/models`).
    sourceSets["androidTest"].assets.srcDir(rootProject.file("../../../fixtures/models"))

    testOptions { execution = "ANDROIDX_TEST_ORCHESTRATOR" }
}

// Decision 108: Kotlin unit tests load the host-JVM build of the Android JNI
// model host (the same Rust + JNI code the AAR ships for arm64-v8a/x86_64).
val hostModelLibrary = layout.buildDirectory.file("native/" + System.mapLibraryName("traverse_android_host"))
val buildHostModelLibrary by tasks.registering(Exec::class) {
    description = "Builds traverse-android-host for the host JVM (Kotlin unit tests)."
    commandLine(
        "bash",
        rootProject.file("../../../scripts/build_android_host_jvm.sh").absolutePath,
        hostModelLibrary.get().asFile.absolutePath,
    )
}
tasks.withType<Test>().configureEach {
    dependsOn(buildHostModelLibrary)
    systemProperty("traverse.android.host.library", hostModelLibrary.get().asFile.absolutePath)
}

dependencies {
    implementation("com.dylibso.chicory:runtime:1.7.5")
    implementation("com.dylibso.chicory:wasm:1.7.5")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.1")
    testImplementation("junit:junit:4.13.2")
    testImplementation("com.dylibso.chicory:wabt:1.7.5")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestUtil("androidx.test:orchestrator:1.5.1")
}

mavenPublishing {
    coordinates("com.traverse-framework", "traverse-embedder", version.toString())

    pom {
        name.set("Traverse Embedder")
        description.set("Traverse embedder-api/1.1.0 public Kotlin/Android boundary.")
        inceptionYear.set("2026")
        url.set("https://github.com/traverse-framework/traverse/")

        licenses {
            license {
                name.set("The Apache License, Version 2.0")
                url.set("http://www.apache.org/licenses/LICENSE-2.0.txt")
                distribution.set("http://www.apache.org/licenses/LICENSE-2.0.txt")
            }
        }

        developers {
            developer {
                id.set("traverse-framework")
                name.set("Traverse Framework")
                url.set("https://github.com/traverse-framework/")
            }
        }

        scm {
            url.set("https://github.com/traverse-framework/traverse/")
            connection.set("scm:git:git://github.com/traverse-framework/traverse.git")
            developerConnection.set("scm:git:ssh://git@github.com/traverse-framework/traverse.git")
        }
    }

    // automaticRelease = true is required: the default (false) uploads the
    // bundle to Central Portal but leaves it pending, requiring someone to
    // manually click "Publish" on central.sonatype.com — confirmed on the
    // real v0.13.0 tag push, whose bundle sat unreleased (404 on
    // repo1.maven.org) despite the Gradle task reporting success.
    publishToMavenCentral(automaticRelease = true)
    signAllPublications()
}
