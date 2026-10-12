cmake_minimum_required(VERSION 3.25)
project(fooyin_tta VERSION 0.1.1 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
include(CTest)
include(GNUInstallDirs)

find_package(Fooyin 0.13.1 CONFIG REQUIRED)
find_package(Threads REQUIRED)
find_program(CARGO_EXECUTABLE cargo REQUIRED)

set(TTA_PLUGIN_INSTALL_DIR "${FOOYIN_PLUGIN_INSTALL_DIR}" CACHE PATH "Directory for the fooyin plugin")
set(FOOYIN_PLUGIN_INSTALL_DIR "${TTA_PLUGIN_INSTALL_DIR}")
set(TTA_CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo")
set(TTA_RUST_LIBRARY "${TTA_CARGO_TARGET_DIR}/release/libfooyin_tta.a")

add_custom_command(
    OUTPUT "${TTA_RUST_LIBRARY}"
    COMMAND "${CARGO_EXECUTABLE}" build --manifest-path "${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml"
            --release --locked --offline --lib --target-dir "${TTA_CARGO_TARGET_DIR}"
    DEPENDS Cargo.toml Cargo.lock src/lib.rs src/codec.rs src/ffi.rs
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}"
    COMMENT "Building the Rust TTA decoder"
    VERBATIM
)
add_custom_target(tta_rust_build DEPENDS "${TTA_RUST_LIBRARY}")
add_library(tta_rust STATIC IMPORTED)
set_target_properties(tta_rust PROPERTIES IMPORTED_LOCATION "${TTA_RUST_LIBRARY}")
add_dependencies(tta_rust tta_rust_build)

create_fooyin_plugin(tta
    JSON_IN plugin/tta.json.in
    SOURCES plugin/ttaplugin.cpp plugin/ttaplugin.h include/fooyin_tta.h
    DEPENDS Fooyin::Core tta_rust Threads::Threads ${CMAKE_DL_LIBS}
)
target_include_directories(tta PRIVATE include)
# The SDK's helper assumes fooyin's own build-directory variables. Use the
# actual imported library directory for a standalone/user-prefix installation.
set_target_properties(tta PROPERTIES INSTALL_RPATH "$<TARGET_FILE_DIR:Fooyin::Core>")
if(UNIX AND NOT APPLE)
    target_link_libraries(tta PRIVATE m)
    target_link_options(tta PRIVATE "LINKER:--exclude-libs,ALL" "LINKER:--no-undefined")
endif()

install(FILES LICENSE LICENSES/tta-reference.txt DESTINATION "${CMAKE_INSTALL_DATADIR}/licenses/fooyin-tta")

if(BUILD_TESTING)
    add_test(NAME rust_decoder
        COMMAND "${CARGO_EXECUTABLE}" test --manifest-path "${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml"
                --locked --offline --target-dir "${TTA_CARGO_TARGET_DIR}")
    add_executable(tta_plugin_test tests/plugin_test.cpp)
    target_link_libraries(tta_plugin_test PRIVATE Fooyin::Core)
    add_dependencies(tta_plugin_test tta)
    add_test(NAME plugin_integration
        COMMAND tta_plugin_test "$<TARGET_FILE:tta>" "${CMAKE_CURRENT_SOURCE_DIR}/tests/fixtures")
endif()
