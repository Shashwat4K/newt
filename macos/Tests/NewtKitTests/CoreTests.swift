import XCTest
@testable import NewtKit

final class CoreTests: XCTestCase {
    func testCoreVersionCrossesTheABIIntact() {
        let version = Core.version
        XCTAssertFalse(version.isEmpty)
        XCTAssertTrue(version.contains("."))
    }
}
