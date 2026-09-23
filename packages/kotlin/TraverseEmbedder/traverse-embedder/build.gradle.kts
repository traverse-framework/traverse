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
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}

dependencies {
    implementation("com.dylibso.chicory:runtime:1.7.5")
    implementation("com.dylibso.chicory:wasm:1.7.5")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.1")
    testImplementation("junit:junit:4.13.2")
    testImplementation("com.dylibso.chicory:wabt:1.7.5")
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

    publishToMavenCentral()
    signAllPublications()
}
