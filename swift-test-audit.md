# Swift test audit

Scores: **5** focused, fast and very likely to catch regressions; **4** fast and valuable, or the only native coverage of a
specified behavior; **3** moderate cost or value; **2** slow and only moderately likely to catch regressions; **1** slow or
unlikely to catch regressions (duplicated by Rust coverage, broad sweeps). Times are per test from the last full run before
the cleanup; a test that runs first in its class also pays one-time warm-up.

Before: 1073 XCTest tests, 307 s of test time. After: 912 tests (estimated 109 s).

Deleted: every score 1, score 2 at 0.3 s or more, and score 3 at 0.5 s or more. The 48 Swift Testing tests
(Core Text provider, render registry, font catalog, bundled fonts, document formats) run in parallel in under 0.5 s; they
all score 5 and are kept.

| Score | Tests | Deleted |
| --- | --- | --- |
| 5 | 638 | 0 |
| 4 | 229 | 0 |
| 3 | 72 | 27 |
| 2 | 63 | 63 |
| 1 | 71 | 71 |

## EVCompactStyleControlsTests
`src/mac/Editor/Tests/EVCompactStyleControlsTests.swift` — 29 tests, 25.6 s; deleting 27

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 1 | 0.82 | testStyleTypeIconsAndScriptControlsShareCompactRows | deleted | 0.82s; style inspector area (overlapping coverage) |
| 1 | 0.65 | testTableStylesExposePlacementAndExplainSourceOwnedCellAlignment | deleted | 0.65s; style inspector area (overlapping coverage) |
| 1 | 0.66 | testDefaultParagraphClearsCharacterParentAndUnchangedMenusAreRetained | deleted | 0.66s; style inspector area (overlapping coverage) |
| 1 | 1.06 | testFontSizeUnitConversionTracksBasedOnParagraphAndKeepsDeclaration | deleted | 1.06s; style inspector area (overlapping coverage) |
| 1 | 0.78 | testFontSizePercentageDraftsRequireIntegersWithinBoundsAndStepperUsesPercent | deleted | 0.78s; style inspector area (overlapping coverage) |
| 1 | 0.87 | testFontSizeUnitConversionRoundsAndClampsOnlyThePercentageDeclaration | deleted | 0.87s; style inspector area (overlapping coverage) |
| 1 | 0.75 | testFirstClickOnInheritedFontSizeUnitActivatesPercentageAsOneUndoGesture | deleted | 0.75s; style inspector area (overlapping coverage) |
| 1 | 0.75 | testInheritedFieldsAreEmptyUntilOverrideAndUncheckingRestoresInheritance | deleted | 0.75s; style inspector area (overlapping coverage) |
| 1 | 0.79 | testFontFaceUsesOneInheritanceCheckbox | deleted | 0.79s; style inspector area (overlapping coverage) |
| 1 | 0.80 | testFirstClickOnInheritedEmphasisTogglesAndUndoesAsOneAction | deleted | 0.80s; style inspector area (overlapping coverage) |
| 3 | 0.72 | testFirstClickOnInheritedStepperIncrementsAndUndoesAsOneAction | deleted | first-click activation as one undo |
| 1 | 1.55 | testFirstMouseClickOnInheritedFontFaceOpensItsNativePopup | deleted | 1.55s; style inspector area (overlapping coverage) |
| 1 | 1.11 | testFirstMouseClickOnInheritedSizeFocusesItsNativeTextEditor | deleted | 1.11s; style inspector area (overlapping coverage) |
| 1 | 1.39 | testFirstMeasurementClickSelectsAllOnFocusAndReentry | deleted | 1.39s; style inspector area (overlapping coverage) |
| 4 | 0.01 | testInheritedNativeControlReceivesTheOriginalMouseDownOnlyAfterActivation | kept | first-click activation |
| 1 | 0.77 | testParagraphControlsUseCoreEnumValuesForDisplayAndNativeActions | deleted | 0.77s; style inspector area (overlapping coverage) |
| 2 | 0.58 | testDefaultThemeSwatchAndPreviewUpdateWithoutMutatingSourceOrStyle | deleted | 0.58s; style inspector area (overlapping coverage) |
| 1 | 0.85 | testExplicitAndInheritedBlackRemainBlackUntilNativeDefaultAction | deleted | 0.85s; style inspector area (overlapping coverage) |
| 1 | 0.88 | testNativeLightFaceThenBoldCommitsSourceBackedStyle | deleted | 0.88s; style inspector area (overlapping coverage) |
| 1 | 0.85 | testPrimaryFamilyAndFaceKeepOrderedFallbackTail | deleted | 0.85s; style inspector area (overlapping coverage) |
| 4 | 1.27 | testFamilySelectionKeepsLightFaceAndFallbacksThroughDuplicateNativeCallbacksAsOneUndo | kept | font family selection |
| 1 | 1.04 | testTypedFamilyUsesRegularWhenPriorNamedFaceDoesNotExist | deleted | 1.04s; style inspector area (overlapping coverage) |
| 1 | 0.70 | testUnmatchedFaceAndUnavailableFamilyDoNotSelectAnUnrelatedFirstFace | deleted | 0.70s; style inspector area (overlapping coverage) |
| 1 | 0.95 | testRejectedFamilyMutationRestoresCommittedComboAndActiveFieldEditor | deleted | 0.95s; style inspector area (overlapping coverage) |
| 1 | 1.14 | testNativeFallbackPopoverOrdersAddsRemovesAppliesAndCancels | deleted | 1.14s; style inspector area (overlapping coverage) |
| 1 | 0.62 | testEveryNumericControlHasAnAdjacentNativeStepperAndInheritedValue | deleted | 0.62s; style inspector area (overlapping coverage) |
| 1 | 0.70 | testParagraphPanelOmitsSideIndentsAndPreservesLoadedValues | deleted | 0.70s; style inspector area (overlapping coverage) |
| 1 | 1.03 | testLineSpacingDraftPreservesIntermediateDecimalTextAndCaretUntilEditingEnds | deleted | 1.03s; style inspector area (overlapping coverage) |
| 3 | 1.51 | testNumericFieldDraftAndLineSpacingPrecisionAndKindTransitions | deleted | draft validation |

## EVCodeBlockLanguageTests
`src/mac/Editor/Tests/EVCodeBlockLanguageTests.swift` — 5 tests, 25.5 s; deleting 2

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 1.71 | testNativeCodeLanguageMenuEscapeAndNoneSelectionPreserveCaretAndUndo | kept | only real native menu tracking of the language picker |
| 5 | 0.08 | testPreviewLanguageFurnitureAndLocalUndoableSelection | kept | 0.08s |
| 3 | 0.33 | testSelectorHoverPaintsBeneathCodeWithoutChangingGeometry | kept | only pixel check of selector opacity/z-order |
| 2 | 0.97 | testCodeLanguageMenuKeepsScrolledViewportAwayFromCaretDuringSyntaxRefresh | deleted | duplicates Rust markdown_code_languages scroll tests |
| 1 | 22.43 | testDemoNoneToCLeavesVisibleFrameAndScrollingUsable | deleted | 22s sweep of 12 demo scenarios; Rust covers language-change scroll policy |

## EVVariableFontIntegrationTests
`src/mac/Editor/Tests/EVVariableFontIntegrationTests.swift` — 6 tests, 22.2 s; deleting 5

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.71 | testNamedRecursiveChoiceSavesPortableFamilyFaceAndCoordinates | deleted | keeps portable variable-font persistence |
| 3 | 0.37 | testSavedPortableFlightlineRequestExposesVariableControlsWithoutAnEdit | kept | only variable-font control test |
| 1 | 5.12 | testFlightlineVariablePresetsSlidersAndSeparateItalicDesign | deleted | 5s; second variable-font sweep |
| 1 | 0.76 | testAxisSliderKeepsBaseWeightWhenNoCoordinatesWereSaved | deleted | 0.76s; style inspector area (overlapping coverage) |
| 1 | 14.20 | testRecursivePresetsAndEveryAxisReachRenderedFontsWithoutSourceEdits | deleted | 14s exhaustive axis sweep |
| 2 | 1.08 | testAllAxisLabelsFitAtMinimumWindowSizeAndStaticFontsRemoveRows | deleted | layout cosmetics |

## EVStyleEditorTests
`src/mac/Editor/Tests/EVStyleEditorTests.swift` — 20 tests, 17.4 s; deleting 16

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 0.45 | testStylePickerOmitsEmptySectionsAndSingleSectionHeadings | deleted | menu-construction detail |
| 4 | 0.72 | testCoordinatorReusesOneModelessWindowAndRetargetsByStableIdentity | kept | single modeless inspector |
| 1 | 1.19 | testTitleFollowsStylesheetAndThemeChangesInTheOpenInspector | deleted | 1.19s; style inspector area (overlapping coverage) |
| 1 | 1.55 | testDocumentPickerKeepsSameNamedStyleAndFallsBackOnlyWhenMissing | deleted | 1.55s; style inspector area (overlapping coverage) |
| 2 | 2.70 | testDocumentPickerEditsEachThemeFamilyWithIndependentUndoAndPreservesSource | deleted | 2.7s |
| 4 | 0.69 | testStyleMutationIsCoreOwnedSourcePreservingMultiViewAndUndoable | kept | inspector edits are core-owned |
| 4 | 0.69 | testContinuousStyleFieldEditsCoalesceIntoOneCoreUndoUnit | kept | undo coalescing |
| 3 | 0.68 | testExplicitNormalValueAndUseInheritedRemainDistinct | deleted | sparse declarations |
| 1 | 0.74 | testTabsAndBaseParentRestrictionsFollowCoreRolesAndCapabilities | deleted | 0.74s; style inspector area (overlapping coverage) |
| 1 | 0.66 | testExternalDisplayNameAndFollowingStyleCommitPreserveStableSelection | deleted | 0.66s; style inspector area (overlapping coverage) |
| 4 | 0.67 | testStaleCallbackIsRejectedAndRefreshesCommittedValues | kept | stale target safety |
| 2 | 0.59 | testThemeEditorRemainsUsableAfterItsContextWindowCloses | deleted | 0.59s; style inspector area (overlapping coverage) |
| 2 | 0.59 | testClosingContextDoesNotRetargetThemeToAnotherDocument | deleted | 0.59s; style inspector area (overlapping coverage) |
| 2 | 0.59 | testClosingOneViewKeepsThemeSettingsIndependentOfOtherViews | deleted | 0.59s; style inspector area (overlapping coverage) |
| 2 | 0.58 | testModelessWindowDoesNotPreventTargetDocumentEditing | deleted | 0.58s; style inspector area (overlapping coverage) |
| 1 | 0.65 | testCoreTextPreviewResolvesThemeForegroundAndRetainsContextualBorderColors | deleted | 0.65s; style inspector area (overlapping coverage) |
| 1 | 0.65 | testCoreTextPreviewResolvesSystemFontAndUsesCurrentThemeCanvas | deleted | 0.65s; style inspector area (overlapping coverage) |
| 1 | 0.90 | testCharacterPropertyScrollerStartsWithControlsVisible | deleted | 0.90s; style inspector area (overlapping coverage) |
| 2 | 1.34 | testDarkAppearancePropagatesToEditorAndKeepsCurrentThemeCanvas | deleted |  |
| 1 | 0.80 | testParagraphPreviewCoreTextGeometryRespondsToResolvedLayoutProperties | deleted | 0.80s; style inspector area (overlapping coverage) |

## EVTableEditingTests
`src/mac/Editor/Tests/EVTableEditingTests.swift` — 10 tests, 16.9 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testInsertTableCountsBodyRowsAndRestoresExactBytesWithUndo | kept | 0.06s |
| 5 | 0.06 | testNativeStructuralActionsRetainAlignmentAndUndoExactSpelling | kept | 0.06s |
| 5 | 0.06 | testHoverDismissesWhenSelectionChangesAndContextActionsNameTheirTarget | kept | 0.06s |
| 5 | 0.08 | testRowHoverWorksOnBothSidesWithoutChangingSelectionAndKeepsItsTargetIntoWidget | kept | 0.08s |
| 5 | 0.08 | testRowHoverEdgesCornersAndClippingUseActualTableBoundaries | kept | 0.08s |
| 5 | 0.08 | testRowHoverDoesNotShowWidgetsWithPreviewOff | kept | 0.08s |
| 5 | 0.06 | testTableGeometryAndCellSelectionRemainPortableAndDisableBlockControls | kept | 0.06s |
| 4 | 1.75 | testIdleTableWidthsRefineOnWorkerAndStopWithoutChangingSourceOrCaret | kept | only native table-width worker test |
| 4 | 0.15 | testNativeCutPasteRoundTripsStyledRectangleWithBreaksAndEmptyCells | kept | 0.15s |
| 1 | 14.51 | testGiantCellEndMotionAndTypingKeepNativeCaretVisible | deleted | 14.5s; refinement convergence now covered in Rust |

## EVStyleEditorTrackingTests
`src/mac/Editor/Tests/EVStyleEditorTrackingTests.swift` — 14 tests, 16.9 s; deleting 13

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 1 | 1.32 | testDocumentActivationRetargetsCurrentStyleWithoutStealingFocusAndCancelsOldFollowing | deleted | 1.32s; style inspector area (overlapping coverage) |
| 1 | 0.94 | testActivationSwitchesStyleFamiliesAndLeavesStandaloneCodeSettingsIndependent | deleted | 0.94s; style inspector area (overlapping coverage) |
| 1 | 2.11 | testNativeEditStylesMenuAndCommandOpenTheCurrentCaretStyle | deleted | 2.11s; style inspector area (overlapping coverage) |
| 1 | 1.35 | testMarkdownCodeModeTransitionRetargetsOpenInspectorAndKeepsFollowing | deleted | 1.35s; style inspector area (overlapping coverage) |
| 1 | 0.99 | testDelayedCodeSyntaxPreservesExplicitStyleChoiceUntilCaretMoves | deleted | 0.99s; style inspector area (overlapping coverage) |
| 4 | 1.12 | testCaretFollowingPrefersNamedCharactersAndManualPickerSurvivesRefreshAndStyleEdits | kept | inspector caret following |
| 3 | 0.88 | testTrackingIsBoundToTheTargetSurfaceAndStopsWhenTargetOrPanelCloses | deleted | closed-target safety |
| 1 | 1.44 | testMixedSelectionsFollowTheirUniformParagraphOrBaseAndEditStylesUsesBase | deleted | 1.44s; style inspector area (overlapping coverage) |
| 1 | 2.04 | testMarkdownSemanticStylesFollowTheCaretInNormalAndInsertModes | deleted | 2.04s; style inspector area (overlapping coverage) |
| 1 | 0.73 | testLinkStyleSelectionAndMixedCharacterStylesUseTheSameInspectorPolicy | deleted | 0.73s; style inspector area (overlapping coverage) |
| 1 | 1.14 | testPendingCharacterChangesFollowWithoutMovingTheInsertCaret | deleted | 1.14s; style inspector area (overlapping coverage) |
| 1 | 0.79 | testCollapsedCaretModeChangeResumesFollowingTheInsertionStyle | deleted | 0.79s; style inspector area (overlapping coverage) |
| 3 | 1.20 | testCodeTrackingKeepsTheGlobalSettingsTargetAndStandaloneSettingsStopFollowing | deleted | code global target |
| 1 | 0.81 | testSelectionFollowingCoalescesQueriesAndCancelsPendingWork | deleted | 0.81s; style inspector area (overlapping coverage) |

## EVBackgroundLayoutTests
`src/mac/Editor/Tests/EVBackgroundLayoutTests.swift` — 5 tests, 10.2 s; deleting 4

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 2.14 | testWorkerWarmsThreePagesWithoutChangingVisiblePresentation | deleted | slow; warm-up is an optimization |
| 4 | 1.10 | testQueuedWorkCancelsForEditsMetricsResizeAndClose | kept | worker cancellation correctness |
| 3 | 1.09 | testCompositionPausesWorkAndCancellationResumesWithoutEditingSource | deleted | worker/IME interaction |
| 2 | 1.33 | testIdleGlyphRetentionDoesNotGrowWithUnrelatedDocumentSuffix | deleted | slow bound check |
| 1 | 4.54 | testOrdinaryAndRapidPagingCompareTheSameNativeFixture | deleted | 4.5s; paging policy is core-tested |

## EVLineNumberGutterTests
`src/mac/Editor/Tests/EVLineNumberGutterTests.swift` — 21 tests, 7.3 s; deleting 10

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.22 | testGutterDragSelectsWholeRowsAndKeepsItsCursorAcrossText | kept | 0.22s |
| 4 | 0.24 | testGutterDragAutoscrollIsVerticalAndRetainsInitialRow | kept | 0.24s |
| 4 | 0.29 | testParagraphMarginSelectionIncludesPaddingAndWrappedRowsWithoutNumbers | kept | 0.29s |
| 4 | 0.26 | testOrdinarySelectionCursorStaysIBeamAcrossMarginsAndOutsideViewInEveryMode | kept | 0.26s |
| 3 | 0.78 | testListMarkerMarginsAndTableControlsUseExactCoreHotZones | deleted | gutter hot zones |
| 2 | 0.42 | testRightToLeftListLabelSelectsItsRowWithoutClaimingTheRightMargin | deleted | gutter hit-test cosmetics |
| 3 | 0.45 | testScrolledMarginHitTestingUsesDocumentCoordinatesAndFixedNumberColumn | kept | 0.45s |
| 4 | 0.23 | testExNumberReservesRealChromeAndPreservesSourceAndHistory | kept | 0.23s |
| 4 | 0.25 | testProportionalDigitsCurrentStyleAndDecimalThresholdCache | kept | 0.25s |
| 4 | 0.22 | testThemeForegroundChangesRefreshPaintWithoutMeasuringDigits | kept | 0.22s |
| 4 | 0.20 | testBuiltinGutterFlagsKeepSameNamedContentStylesIndependent | kept | 0.20s |
| 2 | 0.40 | testFoldedRowShowsFirstNumberAndUsesCurrentStyleForItsSecondSourceLine | deleted | line numbers are core-computed |
| 4 | 0.24 | testPhysicalWrappedBandKeepsItsCenterWhenTheViewportScrollsAndShrinksToFit | kept | 0.24s |
| 2 | 0.53 | testStartAndEndAlignmentHonorExplicitRTLParagraphDirection | deleted | slow cosmetic gutter RTL |
| 2 | 0.40 | testPhysicalNumbersStayIndependentOfVisualCommandLineModeAndSplitCopiesOption | deleted | view option is core-tested |
| 4 | 0.25 | testTranslucentGutterBackgroundPaintsEachLineBoxOnce | kept | 0.25s |
| 2 | 0.33 | testOverlappingNegativeMarginsDoNotRepaintTheBaseBackground | deleted | pixel-seam cosmetics |
| 2 | 0.34 | testFractionalScrollAndDeviceScalesLeaveNoBackgroundOrRightBorderSeams | deleted | pixel-seam cosmetics |
| 2 | 0.48 | testTranslucentBorderCornersAndShortBoxesPaintEachPixelOnce | deleted | pixel-seam cosmetics |
| 2 | 0.41 | testHorizontalBordersReachCanvasEdgesWithZeroVerticalBorders | deleted | pixel-seam cosmetics |
| 2 | 0.33 | testEmptyLineBoxesDoNotGainPixelCoverageAtFractionalCanvasEdges | deleted | pixel-seam cosmetics |

## EVBlockStyleEditorTests
`src/mac/Editor/Tests/EVBlockStyleEditorTests.swift` — 11 tests, 7.2 s; deleting 9

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 0.32 | testContainerCategoryAndBlockTabKeepExistingSelectionRules | deleted | 0.32s; style inspector area (overlapping coverage) |
| 3 | 0.35 | testBlockPropertiesUseInheritedControlsAndOneUndoGesture | kept |  |
| 1 | 0.73 | testChangingToACharacterStyleEndsEditingInTheHiddenBlockTab | deleted | 0.73s; style inspector area (overlapping coverage) |
| 2 | 0.31 | testNativeBlockBackgroundAndBorderPickersCommitColors | deleted | 0.31s; style inspector area (overlapping coverage) |
| 3 | 0.27 | testNestedContainerPreviewUsesCoreBoxesAndRespondsToPaddingAndBorders | kept | 0.27s; style inspector area (overlapping coverage) |
| 2 | 0.31 | testPaddingAndBorderNativeInputsRejectNegativeValuesButMarginsAcceptThem | deleted | 0.31s; style inspector area (overlapping coverage) |
| 1 | 0.65 | testBlockGridOrdersParametersAndSidesAndAlignsLockWithBackground | deleted | 0.65s; style inspector area (overlapping coverage) |
| 3 | 0.73 | testLockNormalizesFirstNonzeroValuesAsOneUndoGestureAndUnlockPreservesThem | deleted | padlock rule |
| 1 | 0.98 | testLinkedNativeFieldsSteppersColorsAndOverridesUpdateOnlyTheirParameter | deleted | 0.98s; style inspector area (overlapping coverage) |
| 1 | 0.60 | testUnspecifiedBorderColorsFollowTextInControlsAndPreviewUntilOverridden | deleted | 0.60s; style inspector area (overlapping coverage) |
| 1 | 1.93 | testThemeUndoRetainsLinkSettingAndRestoresTheWholeRow | deleted | 1.93s; style inspector area (overlapping coverage) |

## EVPointerSelectionTests
`src/mac/Editor/Tests/EVPointerSelectionTests.swift` — 10 tests, 5.5 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.15 | testSingleClickPreservesModeAndFollowingCommandOrTextInput | kept | 0.15s |
| 4 | 0.11 | testBothCharacterHalvesHitTheSameNormalCellButInsertKeepsNearestBoundaries | kept | 0.11s |
| 4 | 0.20 | testNormalDragIncludesClickedGraphemeFromEitherHalfInBothDirectionsAndOnReversal | kept | 0.20s |
| 5 | 0.05 | testInsertDragRetainsTheBoundaryChosenOnTheRightHalf | kept | 0.05s |
| 2 | 4.53 | testOffscreenDragKeepsItsAnchorAndVisibleSelectionAcrossLongDocuments | deleted | 4.5s; anchor retention core-tested |
| 5 | 0.05 | testDoubleClickUsesPortableWordBoundariesAndDoesNotEditSource | kept | 0.05s |
| 4 | 0.15 | testDoubleClickDragKeepsWholeWordsThroughDirectionChanges | kept | 0.15s |
| 5 | 0.09 | testWordDragRetainsGranularityDuringAutoscrollThenNewClickResetsIt | kept | 0.09s |
| 5 | 0.05 | testTripleClickUsesSelectedVisualOrPhysicalLinePolicy | kept | 0.05s |
| 5 | 0.10 | testDoubleClickFromInsertOrReplaceSelectsTheHitWordBeforeTyping | kept | 0.10s |

## EVDocumentHostEffectsTests
`src/mac/AppShellTests/EVDocumentHostEffectsTests.swift` — 22 tests, 4.9 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testWindowInstallsItselfAsTypedDocumentHost | kept | 0.01s; file/launch/input safety |
| 5 | 0.02 | testCurrentFullWriteUsesNativeSaveAndAcknowledgesExactRevision | kept | 0.02s; file/launch/input safety |
| 5 | 0.03 | testFailedWriteDoesNotAcknowledgeOrCloseWriteQuitWindow | kept | 0.03s; file/launch/input safety |
| 5 | 0.03 | testWriteQuitClosesOnlyAfterSuccessfulAcknowledgement | kept | 0.03s; file/launch/input safety |
| 5 | 0.04 | testSaveAsUsesExplicitPathAndMovesNativeDocumentOnlyAfterSuccess | kept | 0.04s; file/launch/input safety |
| 5 | 0.05 | testSaveAsPreservesSourceFormatAndBytesRegardlessOfFilenameExtension | kept | 0.05s; file/launch/input safety |
| 5 | 0.04 | testXitSkipsWriteWhenCleanAndSavesBeforeClosingWhenDirty | kept | 0.04s; file/launch/input safety |
| 5 | 0.01 | testDirtyEditNewAndQuitAllRespectForceGuardBeforeNativeMutation | kept | 0.01s; file/launch/input safety |
| 5 | 0.02 | testWriteAllIncludesCurrentUnregisteredDocumentAndUsesExactSave | kept | 0.02s; file/launch/input safety |
| 5 | 0.02 | testDirtyQuitIsRejectedButForcedQuitClosesWithoutSaving | kept | 0.02s; file/launch/input safety |
| 5 | 0.03 | testStaleWritesFailAndAlternateAndRangedWritesPreserveBindingAndSavePoint | kept | 0.03s; file/launch/input safety |
| 5 | 0.08 | testSuccessfulQuitVariantsNotifyApplicationOnlyAfterClosingWindow | kept | 0.08s; file/launch/input safety |
| 5 | 0.02 | testSplitPaneQuitAndOrdinaryWindowCloseDoNotRequestApplicationQuit | kept | 0.02s; file/launch/input safety |
| 4 | 0.34 | testDirectoryEditPreservesForceAndOnlyReplacesAfterSelection | kept | 0.34s; file/launch/input safety |
| 4 | 0.38 | testDirectoryOpenKeepsOriginalPaneWhenFocusChanges | kept | 0.38s; file/launch/input safety |
| 4 | 0.36 | testDirectoryVerticalSplitKeepsOrientationAfterPickingFile | kept | 0.36s; file/launch/input safety |
| 5 | 0.20 | testDirectoryCapitalEditOpensSelectedFileInNewWindow | kept | 0.20s; file/launch/input safety |
| 4 | 1.97 | testDirectorySelectionRejectsChangedTargetAndClosedWindow | kept | only stale-target check for the directory picker |
| 3 | 0.52 | testDirectoryCancellationIsSilentAndStopsQueuedCommands | deleted |  |
| 5 | 0.21 | testDirectoryCancellationStopsSourceBeforeNextLine | kept | 0.21s; file/launch/input safety |
| 4 | 0.49 | testDirectoryPathsResolveDotRelativeHomeAndSymlinkBeforePicker | kept | 0.49s; file/launch/input safety |
| 5 | 0.02 | testExplicitFilePathOpensWithoutDirectoryPicker | kept | 0.02s; file/launch/input safety |

## EVBlockBackgroundDrawingTests
`src/mac/Editor/Tests/EVBlockBackgroundDrawingTests.swift` — 5 tests, 4.5 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.15 | testQuotedTableHasOneContinuousOuterBorderAndBackground | kept | 0.15s |
| 1 | 3.80 | testTableBorderColorsRefreshThroughThemeInspectorAndReachNativePixels | deleted | 3.8s via inspector; border paint covered elsewhere |
| 4 | 0.20 | testNestedQuoteBackgroundsCompositeOnceOverTheirParents | kept | 0.20s |
| 4 | 0.28 | testCodeContainerBackgroundUpdatesInDocumentAndStylePreview | kept | 0.28s |
| 5 | 0.05 | testPartialRepaintOfTranslucentBackgroundMatchesFullRepaint | kept | 0.05s |

## EVStyleColorWellTests
`src/mac/Editor/Tests/EVStyleColorWellTests.swift` — 6 tests, 4.4 s; deleting 5

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 1 | 0.63 | testOpeningNativePanelShowsCustomColorWithoutEditingOrAddingUndo | deleted | 0.63s; style inspector area (overlapping coverage) |
| 4 | 0.65 | testNativePanelSelectionCommitsOnceAndReopeningShowsCommittedColor | kept | only native color panel test |
| 1 | 0.72 | testSwitchingFromTextToBackgroundColorSeedsCurrentColorWithoutEditingText | deleted | 0.72s; style inspector area (overlapping coverage) |
| 1 | 0.72 | testFirstClickOnInheritedWellEnablesOverrideAndOpensNativePanel | deleted | 0.72s; style inspector area (overlapping coverage) |
| 1 | 0.70 | testStyleAndReadOnlyTransitionsDisconnectNativePanel | deleted | 0.70s; style inspector area (overlapping coverage) |
| 1 | 0.99 | testNewCodeBlockBackgroundStartsOpaqueAndPreservesChosenAlpha | deleted | 0.99s; style inspector area (overlapping coverage) |

## EVStylePropertyLayoutTests
`src/mac/Editor/Tests/EVStylePropertyLayoutTests.swift` — 4 tests, 4.4 s; deleting 4

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 1 | 1.21 | testTabChangesKeepPreviewAndCloseAtFixedBottomPositions | deleted | 1.21s; style inspector area (overlapping coverage) |
| 1 | 0.96 | testColorCaptionsEnableInheritedColorsWithoutOpeningTheColorPanel | deleted | 0.96s; style inspector area (overlapping coverage) |
| 1 | 0.98 | testOpenTypeDirectionAndNumericCaptionsOnlyEnableTheirProperties | deleted | 0.98s; style inspector area (overlapping coverage) |
| 1 | 1.25 | testMinimumSizeShowsCompleteCaptionsAndUnitsOnBothTabs | deleted | 1.25s; style inspector area (overlapping coverage) |

## EVImageTests
`src/mac/Editor/Tests/EVImageTests.swift` — 17 tests, 4.0 s; deleting 3

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testImageFileLocationsAreRelativeWithoutClimbingThroughRootAndRetainFilenameIdentity | kept | 0.00s |
| 3 | 0.51 | testImageFolderButtonUpdatesOnlyItsDraftAndApplyPreservesTitleAndUndoInBothViews | deleted | 0.51s |
| 3 | 0.53 | testDelayedImagePickerCannotChangeAnotherDraftOrRetargetAChangedDocument | deleted | 0.53s |
| 2 | 0.87 | testNativeImageFilePickerKeepsDraftWhileOpenAndAfterCancellation | deleted | 0.87s |
| 5 | 0.08 | testRemoteImageIsAtomicURLPlaceholderAndOnlyExplicitOpenLaunchesBrowser | kept | 0.08s |
| 4 | 0.17 | testInsertEditRemoveAndUndoPreserveImageSourceInBothViews | kept | 0.17s |
| 4 | 0.14 | testLocalRasterLoadsAsynchronouslyAndKeepsAspectWithinDocumentWidth | kept | 0.14s |
| 3 | 0.49 | testHTMLImageDimensionsUseLocalRasterAndPaintTheirLayoutRectangle | kept | 0.49s |
| 5 | 0.07 | testRemoteHTMLImageUsesExistingPopupAndPreservesAttributes | kept | 0.07s |
| 5 | 0.08 | testSelectedImageCopiesPortableFallbackAndNativeBackspaceIsUndoable | kept | 0.08s |
| 5 | 0.08 | testInsertionArrowNavigationSelectsImageAndCanLeaveItsSelection | kept | 0.08s |
| 4 | 0.16 | testImageToolbarAtUnselectedInsertCaretInsertsNewImageBesideExistingObject | kept | 0.16s |
| 3 | 0.35 | testImageParagraphStyleControlsNativeLabelAndSourceFontsAndBoxes | kept | 0.35s |
| 5 | 0.08 | testNormalArrowsTraverseStandaloneImageInBothDirections | kept | 0.08s |
| 4 | 0.16 | testReloadRefreshesChangedFileAndBrokenPlaceholderWithoutEditingSource | kept | 0.16s |
| 4 | 0.16 | testNestedLinkedImagePrefersImagePopupAndRetainsEditingFocus | kept | 0.16s |
| 5 | 0.07 | testImageBesideCombiningTextKeepsAtomicGeometryAndSourceModeLiteral | kept | 0.07s |

## EVArgumentNavigationTests
`src/mac/Editor/Tests/EVArgumentNavigationTests.swift` — 10 tests, 3.8 s; deleting 4

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.28 | testDefaultLaunchIsLazyAndForcedNextDiscardsOnlyTheLastView | kept | 0.28s |
| 3 | 0.53 | testSplitNavigationRetainsModifiedSharedBufferAndUsesEachCurrentFile | deleted | 0.53s |
| 3 | 0.55 | testEditOutsideListRetainsPositionAndEditInsideListUsesCurrentIdentity | deleted | 0.55s |
| 3 | 0.35 | testWriteNextSavesBeforeNavigationIncludingEndOfListAndReadOnlyFailure | kept | 0.35s |
| 2 | 0.38 | testAlternateWriteNextPreservesBindingAndDirtyGuard | deleted | alternate writes covered by EVNativeExFileTests |
| 4 | 0.27 | testFailedAndStaleOpensPreservePaneAndListPosition | kept | 0.27s |
| 3 | 0.35 | testSplitLaunchKeepsArgumentOrderFocusAndInitialLineOnlyInFirstPane | kept | 0.35s |
| 4 | 0.28 | testEditModifiedBufferInSplitNeedsNoDiscardButReloadStillDoes | kept | 0.28s |
| 2 | 0.56 | testExplicitSplitCountCreatesExtraBlankPanesAndSupportsNoFiles | deleted | split counts covered by EVLaunchArgumentsTests |
| 4 | 0.21 | testNewWindowForBufferInheritsListAndKeepsOriginalModifiedView | kept | 0.21s |

## EVDeleteSelectionTests
`src/mac/Editor/Tests/EVDeleteSelectionTests.swift` — 3 tests, 3.6 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.08 | testBothPhysicalDeleteKeysRemoveAccessibilitySelectionMadeFromNormalMode | kept | 0.08s |
| 5 | 0.04 | testPhysicalDeleteRemovesDoubleClickedWordAndEntersInsertMode | kept | 0.04s |
| 1 | 3.51 | testTripleClickDeleteAtEveryStructuralLineIncludingLastListItem | deleted | 3.5s UI matrix duplicating edit_boundary_matrix |

## EVLaunchArgumentsTests
`src/mac/Editor/Tests/EVLaunchArgumentsTests.swift` — 17 tests, 3.6 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testPortableParserSurvivesSwiftABIAndPreservesLiteralFilenames | kept | 0.00s |
| 5 | 0.00 | testMalformedFlagsProduceUserFacingErrors | kept | 0.00s |
| 4 | 0.16 | testInitialLineUsesPhysicalLinesAndClampsWithoutEditing | kept | 0.16s |
| 3 | 0.37 | testInitialLineCancelsExistingInputWithoutChangingSource | kept | 0.37s |
| 4 | 0.24 | testLaunchOpensFirstMissingFilenameAsNamedEmptyBufferAndKeepsRemainingArguments | kept | 0.24s |
| 4 | 0.15 | testLaunchSplitsExistingFilesAndPositionsOnlyTheFirstFile | kept | 0.15s |
| 4 | 0.25 | testLaunchConstructsFirstViewFromRecoveredDirtyDocument | kept | 0.25s |
| 4 | 0.12 | testEmptyForwardedLaunchReusesWindowAndCreatesBlankAfterLastWindowCloses | kept | 0.12s |
| 4 | 0.14 | testForwardedRelativeFilenamesUseSenderDirectoryAndRetainLazyArgumentList | kept | 0.14s |
| 4 | 0.19 | testForwardedExistingFirstFileDoesNotEagerlyOpenLaterArguments | kept | 0.19s |
| 4 | 0.14 | testForwardedExistingFileKeepsUnsavedInsertTextAndNavigatesInSameView | kept | 0.14s |
| 4 | 0.19 | testForwardedExistingSplitFileFocusesItsPaneWithoutDuplicatingIt | kept | 0.19s |
| 4 | 0.26 | testForwardedMixedSplitLaunchReusesExistingFileAndOpensOnlyNewFilesTogether | kept | 0.26s |
| 4 | 0.13 | testRepeatedCanonicalFileArgumentsNeverCreateDuplicatePanes | kept | 0.13s |
| 2 | 1.00 | testRepeatedExistingArgumentsDoNotConsumeSpaceForNewSplitFiles | deleted | 1.00s |
| 5 | 0.08 | testExplicitSplitCountLimitsArgumentSlotsBeforeDuplicateSuppression | kept | 0.08s |
| 4 | 0.22 | testForwardedSplitOptionsWithoutFilenamesCreateRequestedBlankPanes | kept | 0.22s |

## EVCodeStyleMenuTests
`src/mac/Editor/Tests/EVCodeStyleMenuTests.swift` — 6 tests, 3.5 s; deleting 4

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 1 | 0.84 | testNativeCodeMenusMarkTheActiveSyntaxAndBaseStylesAndValidationUpdatesMovement | deleted | 0.84s; style inspector area (overlapping coverage) |
| 4 | 0.06 | testCodeMenuChecksOneUniformSyntaxSelectionAndClearsCharacterChecksForMixedText | kept | 0.06s; style inspector area (overlapping coverage) |
| 3 | 1.00 | testCodeMenuListsEveryCharacterDefinitionAndEditsTheGlobalTarget | deleted |  |
| 1 | 0.98 | testDeletedSyntaxNameIsRegeneratedAsAnImplicitDefinition | deleted | 0.98s; style inspector area (overlapping coverage) |
| 4 | 0.01 | testChoosingAnUnmaterializedSyntaxNameGeneratesItsAncestry | kept | 0.01s; style inspector area (overlapping coverage) |
| 1 | 0.65 | testCodeEditStylesCommandsTargetBasesAndAssignmentsStayDisabled | deleted | 0.65s; style inspector area (overlapping coverage) |

## EVNativeExFileTests
`src/mac/Editor/Tests/EVNativeExFileTests.swift` — 13 tests, 3.5 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.18 | testFilenameDirectoryMenuUsesNativeCdAndPreservesDocumentState | kept | 0.18s |
| 3 | 0.37 | testNewMarkdownExOpensUsePreferenceButCurrentFileReloadKeepsItsView | kept | 0.37s |
| 3 | 0.49 | testAlternateWritePreservesBytesBindingDirtyStateAndRejectsOverwriteAndReadOnly | kept | 0.49s |
| 4 | 0.26 | testRangedWriteCopiesPhysicalSyntaxAndSaveAsAdoptsOnlyAfterSuccess | kept | 0.26s |
| 4 | 0.23 | testWorkingDirectoryIsSharedByPwdAndRelativeFileCommands | kept | 0.23s |
| 3 | 0.45 | testEditReplacesActivePaneWhileUppercaseEditAddsWindow | kept | 0.45s |
| 4 | 0.28 | testReadUsesActiveSharedViewAndIsOneUndoableChange | kept | 0.28s |
| 4 | 0.14 | testReadRejectsStaleCompletionAndHandlesEmptyBufferWithoutExtraLine | kept | 0.14s |
| 4 | 0.22 | testSourceRunsMappingsEditsNestedFilesAndStopsAtPathLineError | kept | 0.22s |
| 4 | 0.14 | testSourceRejectsConfirmationAndRecursiveFilesWithoutLeavingPrompt | kept | 0.14s |
| 4 | 0.17 | testFileRenamesBindingWithoutWritesOrDirtyChangesAndPreservesOverwriteReview | kept | 0.17s |
| 4 | 0.20 | testOnlyRefusesDirtyLastViewsAndBangKeepsActivePane | kept | 0.20s |
| 3 | 0.35 | testFilePreservesSameNameBaselineAndRefusesAnotherLiveDocument | kept | 0.35s |

## EVViewMarginIntegrationTests
`src/mac/Editor/Tests/EVViewMarginIntegrationTests.swift` — 8 tests, 3.5 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 2.27 | testMarkdownDemoReadBeforeNativeViewAttachmentAppliesMargins | deleted | 2.3s demo load; other margin tests remain |
| 4 | 0.24 | testReplacingAnAttachedDocumentAppliesCurrentThemeStylesAndKeepsSource | kept | 0.24s |
| 5 | 0.09 | testUntitledStartupAppliesViewMarginsBeforeTheFirstPresentationAndAcceptsTyping | kept | 0.09s |
| 4 | 0.28 | testLargeWrappedDocumentAppliesMarginsInItsOnlyInitialLayout | kept | 0.28s |
| 5 | 0.07 | testHiddenLegacyHorizontalScrollbarDoesNotClipTheBottomTextBand | kept | 0.07s |
| 4 | 0.13 | testTypingAndNewlinesKeepTheWholeRowAboveTheBottomMarginInBothScrollbarStyles | kept | 0.13s |
| 3 | 0.31 | testTypingOnAnAlreadyVisibleRowNeverMovesTheDisplayedRows | kept | 0.31s |
| 5 | 0.09 | testViewMarginNotificationUpdatesOpenViewsWithoutChangingTheirDocuments | kept | 0.09s |

## EVFormattingToolbarTests
`src/mac/Editor/Tests/EVFormattingToolbarTests.swift` — 18 tests, 3.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.12 | testCharacterAndTableGroupsHaveRequestedOrderAndStandardGaps | kept | 0.12s |
| 5 | 0.07 | testPreviewParagraphSelectorShowsOnlySupportedAssignmentsAndKeepsCatalogue | kept | 0.07s |
| 4 | 0.12 | testPreviewListCaretShowsItsExactParagraphStyleWithoutOfferingAssignment | kept | 0.12s |
| 3 | 0.34 | testUnderlineAndScriptToolbarActionsRoundTripAndUndo | kept | 0.34s |
| 4 | 0.80 | testMarkdownPreviewTogglePreservesSourceTracksSharedHistoryAndRestoresFocus | kept | Preview toggle |
| 5 | 0.07 | testSelectorsTrackCaretMixedSelectionAssignmentAndUndo | kept | 0.07s |
| 5 | 0.07 | testCharacterCodeTogglesAndCodeBlockAvailability | kept | 0.07s |
| 5 | 0.06 | testMarkdownCodeBlockToolbarTogglesAndRestoresExactSourceWithUndo | kept | 0.06s |
| 4 | 0.25 | testBlockQuoteToolbarTogglesStructureAndPreservesInnerTreatments | kept | 0.25s |
| 4 | 0.25 | testBlockQuoteStaysVisibleButDisabledForCodeBlocksAndMixedSelections | kept | 0.25s |
| 4 | 0.13 | testUndoInDemoMermaidBlockPreservesVisibleTextPosition | kept | 0.13s |
| 5 | 0.06 | testMarkdownOmitsUnsupportedPropertiesAndButtonsShareMenuActions | kept | 0.06s |
| 3 | 0.33 | testNormalCaretTracksInlineTraitsAndTogglesEnterInsertWithoutSourceChanges | kept | 0.33s |
| 4 | 0.16 | testLinkToolbarTracksCaretSplitsTypingAndRemovesSelectedTreatment | kept | 0.16s |
| 5 | 0.06 | testImageToolbarIsActiveDisabledForCurrentOrSelectedImageAndOffAtTypingCaret | kept | 0.06s |
| 4 | 0.22 | testInlineButtonsRetainReadOnlyStateAndDisableLiteralCodeBlockTyping | kept | 0.22s |
| 5 | 0.06 | testListTogglesUseStructuralStateAcrossNestingAndMixedParagraphs | kept | 0.06s |
| 4 | 0.20 | testWindowRefreshKeepsToolbarCurrentWithoutPollingAndFitsNarrowWindows | kept | 0.20s |

## EVCommandLineLauncherTests
`src/mac/BlockingTransportTests/EVCommandLineLauncherTests.swift` — 8 tests, 3.3 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.29 | testLaunchPreservesArgumentsAndWorkingDirectoryThroughExternalSymlink | kept | 0.29s; file/launch/input safety |
| 5 | 0.24 | testNoArgumentsReturnsOnAcceptanceWhileGUIStaysAlive | kept | 0.24s; file/launch/input safety |
| 4 | 0.34 | testForwarderMayExitBeforeOwningGUIAcceptsRequest | kept | 0.34s; file/launch/input safety |
| 4 | 0.31 | testMissingGUIFailedLaunchAndMissingAcknowledgmentFail | kept | 0.31s; file/launch/input safety |
| 4 | 1.04 | testConnectedSilentAndPartialAcknowledgmentsTimeOut | kept | launcher must never report false success |
| 4 | 0.77 | testClosedAndPartialAcknowledgmentsNeverReportSuccess | kept | launcher must never report false success |
| 5 | 0.24 | testRejectionForwardsNonzeroAcceptanceStatus | kept | 0.24s; file/launch/input safety |
| 5 | 0.01 | testInvalidArgumentsAndDirectoriesFailBeforeLaunch | kept | 0.01s; file/launch/input safety |

## EVFileChangeMonitorTests
`src/mac/AppShellTests/EVFileChangeMonitorTests.swift` — 12 tests, 3.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.29 | testInPlaceWritesNotifyOnMainThreadAndCoalesceBurst | kept | 0.29s; file/launch/input safety |
| 5 | 0.16 | testAtomicReplacementReattachesToNewFile | kept | 0.16s; file/launch/input safety |
| 5 | 0.25 | testDeletionAndRecreationContinueWatching | kept | 0.25s; file/launch/input safety |
| 5 | 0.08 | testInitiallyMissingFileIsWatchedThroughItsDirectory | kept | 0.08s; file/launch/input safety |
| 4 | 0.49 | testRemovedContainingDirectoryCanBeRecreatedAndEditedAgain | kept | 0.49s; file/launch/input safety |
| 4 | 0.51 | testMovedContainingDirectoryCanBeRecreatedAndEditedAgain | kept | 0.51s; file/launch/input safety |
| 5 | 0.25 | testSymbolicLinkTargetInAnotherDirectoryCanBeDeletedAndRecreated | kept | 0.25s; file/launch/input safety |
| 5 | 0.09 | testInitiallyBrokenSymbolicLinkObservesTargetCreationElsewhere | kept | 0.09s; file/launch/input safety |
| 5 | 0.25 | testRetargetedSymbolicLinkChainWatchesItsNewTargetDirectory | kept | 0.25s; file/launch/input safety |
| 5 | 0.26 | testSiblingFileChangesDoNotNotify | kept | 0.26s; file/launch/input safety |
| 5 | 0.26 | testStopSuppressesQueuedEventsAndIsIdempotent | kept | 0.26s; file/launch/input safety |
| 5 | 0.26 | testReleasingMonitorStopsWatchingWithoutExplicitStop | kept | 0.26s; file/launch/input safety |

## EVStyleEditorHierarchyTests
`src/mac/Editor/Tests/EVStyleEditorHierarchyTests.swift` — 5 tests, 3.1 s; deleting 4

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.44 | testParentButtonsTraverseFixedBaseStylesWithoutChangingDocumentOrHistory | kept | hierarchy navigation |
| 2 | 0.49 | testNextParagraphButtonNavigatesOnlyARealDifferentParagraph | deleted | 0.49s; style inspector area (overlapping coverage) |
| 2 | 0.48 | testNavigatingAfterLiveFieldEditsKeepsCommittedValuesAndClosesOneUndoGroup | deleted | 0.48s; style inspector area (overlapping coverage) |
| 2 | 0.30 | testGlobalCodeHierarchyNavigationDoesNotPersistOrAddSettingsUndo | deleted | 0.30s; style inspector area (overlapping coverage) |
| 1 | 1.40 | testHeaderLabelsAlignWithNativeControlsAndNavigationButtonsStayBesidePopups | deleted | 1.40s; style inspector area (overlapping coverage) |

## EVWindowCommandTests
`src/mac/AppShellTests/EVWindowCommandTests.swift` — 20 tests, 3.0 s; deleting 2

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.05 | testWindowRestorationAndSynchronousGutterResizingPreserveSavedContentWidth | kept | 0.05s |
| 5 | 0.04 | testFocusCommandsWalkClampAndWrapThroughStackedPanes | kept | 0.04s |
| 5 | 0.03 | testLastAccessedAlternatesBetweenTwoPanes | kept | 0.03s |
| 5 | 0.03 | testRotateMovesEveryPaneAndKeepsTheSameOneFocused | kept | 0.03s |
| 5 | 0.03 | testExchangeAndMoveReorderPanesAndCarryFocus | kept | 0.03s |
| 5 | 0.02 | testHeightCommandsMoveWholeRowsAndEqualizeRestoresTheSplit | kept | 0.02s |
| 5 | 0.03 | testExtremeCountsAllowEditorsToCollapseWithoutOverlappingBars | kept | 0.03s |
| 4 | 0.25 | testDragCollectsBarsAtBothEdgesAndReversesImmediately | kept | 0.25s |
| 5 | 0.07 | testBottomBarIsFixedAndSplitAdmissionUsesOnlyTheCurrentEditorGap | kept | 0.07s |
| 5 | 0.01 | testExactlyOneBarOfRoomAllowsSplitAndTouchingBars | kept | 0.01s |
| 4 | 0.14 | testWindowResizeKeepsCollapsedGapsAndFullBars | kept | 0.14s |
| 5 | 0.10 | testReorderingMovesTheFixedBarRoleAndDraggingPreservesFocus | kept | 0.10s |
| 3 | 0.57 | testEyeClicksAndDragsUseTheNativeGesturePath | deleted | splitter gesture test remains |
| 5 | 0.00 | testVerticalSplitterTranslucentCornersArePaintedOnce | kept | 0.00s |
| 5 | 0.03 | testVerticalSplitterUnderStatusBarOmitsTopCapWhenUpperPaneCollapses | kept | 0.03s |
| 4 | 0.15 | testNestedVerticalSplittersClipBordersOnlyAtOuterWindowEdges | kept | 0.15s |
| 4 | 0.72 | testVerticalNativeSplittersPushReverseAndStatusClicksFocusWithoutDragFocus | kept | native splitter gestures |
| 2 | 0.71 | testVerticalSplitStatusHeightsAndTextAlignmentSurviveNarrowingAndWidening | deleted | cosmetic; status layout tested elsewhere |
| 5 | 0.03 | testCloseOthersLeavesOnlyTheFocusedPane | kept | 0.03s |
| 5 | 0.03 | testRequestsFromAForeignSurfaceAreIgnored | kept | 0.03s |

## EVCodeScrollingTests
`src/mac/Editor/Tests/EVCodeScrollingTests.swift` — 4 tests, 3.0 s; deleting 2

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.46 | testIdleFontInvalidationIsRefreshedBeforeWheelAndScrollbarRequests | kept | 0.46s |
| 3 | 0.35 | testRustScrollingPublishesSyntaxForNewVisibleRanges | kept | 0.35s |
| 2 | 0.64 | testRustAndVimScrollingPublishesBeforeTheFirstFrameOrCompletesAfterTimeout | deleted | 0.64s |
| 2 | 1.51 | testRustWheelScrollingRemainsUsableAcrossSyntaxAndGeometryRefreshes | deleted | 1.5s; other code scrolling tests remain |

## EVLinkPopoverTests
`src/mac/Editor/Tests/EVLinkPopoverTests.swift` — 10 tests, 2.9 s; deleting 3

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.23 | testCommandKOpensInsertionOrEditingWithoutChangingExistingLinks | kept | 0.23s |
| 4 | 0.26 | testNativeHeadingPickerEscapeKeepsDraftAndSelectionKeepsFragmentAfterTracking | kept | 0.26s |
| 2 | 0.35 | testHeadingSelectionRestorationRetainsTypedDestinationAndNewDraft | deleted | popup draft detail; other link popup tests remain |
| 3 | 0.59 | testURLOnlyInsertionAndInvalidDestinationLeavesDraftAndSourceIntact | deleted | 0.59s |
| 4 | 0.17 | testToolbarUsesSelectedTextAndAppliesOneUndoableLinkInBothViews | kept | 0.17s |
| 2 | 0.79 | testCaretToolbarOpensEmptyDraftAndHeadingComboUsesExactFragments | deleted | 0.79s |
| 4 | 0.28 | testCompactPopupFitsLabelAndRetainsMaximumWidth | kept | 0.28s |
| 5 | 0.07 | testOpeningDestinationDismissesPopupBeforeRouting | kept | 0.07s |
| 4 | 0.10 | testCancelAndStaleSelectionCannotApplyDraft | kept | 0.10s |
| 4 | 0.10 | testCompactPopupAlignsWithLinkAndClosesOnViewRemoval | kept | 0.10s |

## EVSmartQuoteIngressTests
`src/mac/Editor/Tests/EVSmartQuoteIngressTests.swift` — 7 tests, 2.5 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.39 | testEveryNativeTextIngressUsesSmartQuotesInProseAndOneUndoUnit | kept | 0.39s |
| 2 | 1.17 | testCodeSpansAndParagraphsKeepLiteralQuotesAcrossNativeIngress | deleted | slow; literal policy is core-tested |
| 3 | 0.41 | testNormalReplaceAndItsMarkedTextOperandUseTheSameQuotePolicy | kept | 0.41s |
| 4 | 0.15 | testExplicitNativeReplacementUsesTheReplacedContentCodeContext | kept | 0.15s |
| 4 | 0.15 | testNativeReplaceModeCurvesProseAndKeepsCodeLiteral | kept | 0.15s |
| 4 | 0.13 | testAccessibilityWholeValueReplacementUsesSmartQuotesInNormalMode | kept | 0.13s |
| 5 | 0.08 | testRichPasteCurvesProseQuotesAndPreservesCodeQuotesInTheSameFragment | kept | 0.08s |

## EVBlockingEditIntegrationTests
`src/mac/Editor/Tests/EVBlockingEditIntegrationTests.swift` — 8 tests, 2.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.13 | testInitialBlockingLaunchActivatesOnceAndWaitsForDocumentClosure | kept | 0.13s; file/launch/input safety |
| 4 | 0.49 | testConcurrentRelativeAndAliasedCallersReuseExistingDocumentUntilItsLastViewCloses | kept | 0.49s; file/launch/input safety |
| 5 | 0.29 | testSaveAndReloadKeepWaitingAndCanceledDirtyCloseDoesNotFinish | kept | 0.29s; file/launch/input safety |
| 4 | 0.31 | testWriteQuitCompletesOnlyRequestedDocumentWhileOtherFilesRemainOpen | kept | 0.31s; file/launch/input safety |
| 5 | 0.20 | testOpenFailureReturnsFailureWithoutReleasingAnUnrelatedCaller | kept | 0.20s; file/launch/input safety |
| 5 | 0.13 | testMissingFirstFileWaitsForItsNamedEmptyDocumentToClose | kept | 0.13s; file/launch/input safety |
| 5 | 0.06 | testCanceledInitialRecoveryFailsCallerWithoutCreatingAWindow | kept | 0.06s; file/launch/input safety |
| 4 | 0.78 | testCquitPublishesRequestedStatusBeforeClosingAllUnsavedDocuments | kept | :cq status to blocking callers |

## EVPresentationCacheIntegrationTests
`src/mac/Editor/Tests/EVPresentationCacheIntegrationTests.swift` — 9 tests, 2.2 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.41 | testOptionalExportFailuresKeepCurrentGeometryScrollingAndPointerPlacement | kept | 0.41s |
| 5 | 0.07 | testFailedGeometryExportNeverReusesAnOlderLayoutAndDoesNotBlockScrolling | kept | 0.07s |
| 2 | 0.51 | testRepeatedVisibleSelectionsInAgentFileReusePresentationExports | deleted | duplicate of the 20k-line reuse test |
| 3 | 0.48 | testRepeatedVisibleSelectionsInTwentyThousandLinesReusePresentationExports | kept | export reuse in large document |
| 5 | 0.06 | testEditUndoAndRedoInvalidateExportsAndVisibleText | kept | 0.06s |
| 4 | 0.17 | testResizeZoomAndMetricsInvalidateExports | kept | 0.17s |
| 4 | 0.16 | testWhitespaceOptionsAndViewportInvalidateOnlyCurrentMarkerExport | kept | 0.16s |
| 5 | 0.06 | testCompositionRefreshReusesSlicesAndUpdateAndCancellationInvalidateThem | kept | 0.06s |
| 4 | 0.25 | testNamedPaintChangeAndDecorationsMatchFreshExports | kept | 0.25s |

## EVStyleVariantTests
`src/mac/Editor/Tests/EVStyleVariantTests.swift` — 4 tests, 2.1 s; deleting 4

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.85 | testForeignParentsUseDocumentIconsAndRejectCyclesWithoutChangingTheChoiceOrHistory | deleted | cycle rejection |
| 2 | 0.58 | testBaseParagraphOffersOnlyForeignParentsAndNone | deleted | 0.58s; style inspector area (overlapping coverage) |
| 2 | 0.37 | testSharedChromeDeclarationsKeepEachVariantsOwnParent | deleted | 0.37s; style inspector area (overlapping coverage) |
| 2 | 0.32 | testDependentOpenDocumentRefreshesWithoutChangingSourceOrItsHistory | deleted | 0.32s; style inspector area (overlapping coverage) |

## EVGiantLineLayoutTests
`src/mac/Editor/Tests/EVGiantLineLayoutTests.swift` — 4 tests, 2.1 s; deleting 2

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.15 | testWrappedGiantWordRetainsBoundedNativeOverflowGeometry | kept | 0.15s |
| 2 | 0.50 | testWrappedGiantWordAfterShortLinePreservesPrefixAndTail | deleted | overlaps the wrapped giant word test |
| 2 | 0.46 | testBaseParagraphMetricsEditKeepsGiantWordGeometryBoundedAndUndoable | deleted | two giant-line tests remain |
| 4 | 1.02 | testNativeGiantLineKeepsExactVisibleShapingAndRefillsHorizontalGeometry | kept | giant unwrapped line with real shaping |

## EVTextInputClientTests
`src/mac/Editor/Tests/EVTextInputClientTests.swift` — 23 tests, 2.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.13 | testEscapeDiscardsAccentPickerStateWithoutMarkedText | kept | 0.13s; file/launch/input safety |
| 4 | 0.45 | testDirectCancelActionAndControlBracketCancelEveryMarkedTargetLocally | kept | 0.45s; file/launch/input safety |
| 5 | 0.06 | testAccentReplacementCancellationRetainsOriginalLetterAndUndoGroup | kept | 0.06s; file/launch/input safety |
| 5 | 0.05 | testActiveCompositionRoutesEditingKeysThroughAppKitInterpretation | kept | 0.05s; file/launch/input safety |
| 5 | 0.05 | testInsertTextHonorsExplicitUTF16ReplacementAsOneUndoUnit | kept | 0.05s; file/launch/input safety |
| 5 | 0.04 | testEmptyInsertTextDeletesExplicitReplacementRange | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testInsertTextRetargetsAnActiveCompositionWhenRangeIsExplicit | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testSelectedRangeUsesMarkedTextRelativeSelection | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testDocumentCompositionUsesProjectedLayoutGeometryAndAccessibility | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testMalformedUTF16RangesAreRejectedWithoutMutationOrTraps | kept | 0.04s; file/launch/input safety |
| 5 | 0.05 | testDirectCoreCancellationImmediatelyClearsNativeMarkedState | kept | 0.05s; file/launch/input safety |
| 5 | 0.04 | testRejectedExplicitReplacementKeepsNativeAndCoreMarkedStateAligned | kept | 0.04s; file/launch/input safety |
| 5 | 0.05 | testInsertModeCompositionUpdatesCommitsAndCancelsThroughCore | kept | 0.05s; file/launch/input safety |
| 5 | 0.05 | testReplaceModeCompositionUsesCoreAndHonorsExplicitReplacement | kept | 0.05s; file/launch/input safety |
| 4 | 0.33 | testNormalAndEveryVisualModeRejectNativeCompositionAndAccentReplacement | kept | 0.33s; file/launch/input safety |
| 5 | 0.17 | testRepeatedCommandKeysBypassNativeTextInterpretation | kept | 0.17s; file/launch/input safety |
| 5 | 0.05 | testInputContextTracksTextEntryAndDiscardsUnmarkedCandidatesOnModeExit | kept | 0.05s; file/launch/input safety |
| 5 | 0.18 | testStatusLineKeepsNativeInterpretationAndAccentReplacement | kept | 0.18s; file/launch/input safety |
| 5 | 0.05 | testUnicodeCommandOperandsStillReachCoreWithoutNativeComposition | kept | 0.05s; file/launch/input safety |
| 5 | 0.04 | testExCommandLineCompositionIsLocalUntilCommitAndRendersMarkedSelection | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testSearchCommandLineCompositionCommitsViaCommandInputAndCancelsLocally | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testStaleCommandLineDraftCannotOverwriteNewerCoreInput | kept | 0.04s; file/launch/input safety |
| 5 | 0.06 | testMouseDragStartsAndMouseUpStopsSelectionAutoscroll | kept | 0.06s; file/launch/input safety |

## EVCodeStyleLayoutTests
`src/mac/Editor/Tests/EVCodeStyleLayoutTests.swift` — 2 tests, 2.0 s; deleting 2

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 1.34 | testKeywordFontEditsReshapeTwoRustBuffersWithoutChangingSourceOrDocumentHistory | deleted | slow; global code styles covered by EVCodeStylesTests |
| 3 | 0.66 | testDistantRustSyntaxAndFontChangesKeepExactVisibleGeometryAndEstimatedScrollbar | deleted | distant syntax geometry |

## EVPageNavigationTableTests
`src/mac/Editor/Tests/EVPageNavigationTableTests.swift` — 3 tests, 2.0 s; deleting 3

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 0.67 | testPhysicalPageKeysRetraceViewportAcrossTableAndProseWithoutExtraCaretScrolls | deleted | paging is core-tested |
| 2 | 0.60 | testPagingInTablePreservesMiddleColumnAndUsesEqualDistancesInBothDirections | deleted | paging is core-tested |
| 3 | 0.69 | testOversizedRowsPreferAnAvailableVisibleBaselineWithoutChangingPageDistance | deleted | real-metric oversized rows |

## EVApplicationOpenTests
`src/mac/AppShellTests/EVApplicationOpenTests.swift` — 13 tests, 1.9 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.03 | testLaunchReadsFirstDocumentBeforeConstructingItsOnlySurface | kept | 0.03s; file/launch/input safety |
| 5 | 0.03 | testCommandLineLaunchAcknowledgesBeforeReadingAndRequestsForegroundActivation | kept | 0.03s; file/launch/input safety |
| 5 | 0.02 | testInitialEmptyCommandLineLaunchAcknowledgesOnceAndCreatesAnUntitledWindow | kept | 0.02s; file/launch/input safety |
| 5 | 0.07 | testDeferredCommandLineLaunchAcknowledgesBeforeItsQueuedOpen | kept | 0.07s; file/launch/input safety |
| 5 | 0.02 | testCanceledInitialLaunchConstructsOnlyTheRemainingBlankSurface | kept | 0.02s; file/launch/input safety |
| 3 | 0.93 | testOpenReusesTheExactBlankWindowAndPreservesUserGeometry | deleted | other window-reuse tests remain |
| 5 | 0.09 | testAdditionalOpenSelectionCreatesAnotherWindowWithoutReplacingTheFirstFile | kept | 0.09s; file/launch/input safety |
| 4 | 0.30 | testDirtyNonemptyNamedRecoveredOrUnknownDocumentsKeepTheirWindows | kept | 0.30s; file/launch/input safety |
| 5 | 0.17 | testOpenRechecksChangesMadeWhileARecoveryPromptRuns | kept | 0.17s; file/launch/input safety |
| 5 | 0.03 | testFailedReadLeavesTheBlankWindowAvailableForTheNextOpen | kept | 0.03s; file/launch/input safety |
| 5 | 0.08 | testClosedOrSplitWindowIsNotReusedByPendingOpen | kept | 0.08s; file/launch/input safety |
| 5 | 0.04 | testAlreadyOpenFileKeepsItsExistingWindowAndBackend | kept | 0.04s; file/launch/input safety |
| 5 | 0.10 | testOpeningAndReopeningPopulateThePersistedMenuAndClearSurvivesRestart | kept | 0.10s; file/launch/input safety |

## EVCodeEditingStabilityTests
`src/mac/Editor/Tests/EVCodeEditingStabilityTests.swift` — 3 tests, 1.9 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 1.19 | testTypingInScrolledCodePreservesOffCenterCaretBaselineWithAndWithoutRustSyntax | deleted | baseline policy core-tested; two faster siblings remain |
| 3 | 0.31 | testDelayedLargerRustSyntaxPreservesTheTypedCaretBaseline | kept | 0.31s |
| 3 | 0.39 | testLargerSyntaxMovesCaretOnlyEnoughToExposeItsWholeRow | kept | 0.39s |

## EVRecoveryDocumentTests
`src/mac/AppShellTests/EVRecoveryDocumentTests.swift` — 16 tests, 1.9 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testRecoverRetiresOnlySelectedStaleSnapshotAfterWritingOwnedBackup | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testEditAndDeleteOpensDiskContentsAndRemovesStalePromptCandidates | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testReadOnlyAndCancelPreserveStaleRecovery | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testDeleteRejectsLiveAndForeignConflictsBeforeReadingDocument | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testFailedDiskOpenDoesNotDiscardRecovery | kept | 0.01s; file/launch/input safety |
| 4 | 0.98 | testFailedBackupPreservesOldFileForRecoverAndExplicitDiscard | kept | recovery safety |
| 5 | 0.01 | testRecoverLoadsUnsavedBytesAndLeavesOriginalAndOtherOwnerUntouched | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testReadOnlyBlocksAllUnforcedWriteEntryPointsAndNativeSaveCanCancel | kept | 0.01s; file/launch/input safety |
| 5 | 0.02 | testReadOnlyExplicitForceWritesAndAcknowledgesExactSnapshot | kept | 0.02s; file/launch/input safety |
| 5 | 0.10 | testFailedRecoveryRebindingKeepsThePreviousValidSnapshot | kept | 0.10s; file/launch/input safety |
| 5 | 0.19 | testSaveAsRebindsRecoveryAndSubsequentIdleEditUsesNewTarget | kept | 0.19s; file/launch/input safety |
| 5 | 0.17 | testSuccessfulSaveAsKeepsNewRecoveryBindingWhenAcknowledgementIsStale | kept | 0.17s; file/launch/input safety |
| 5 | 0.14 | testFailedURLReadPreservesPendingRecoveryForCurrentDocument | kept | 0.14s; file/launch/input safety |
| 5 | 0.03 | testNativeReadOnlySaveRequiresAndHonorsConfirmation | kept | 0.03s; file/launch/input safety |
| 5 | 0.02 | testForeignSwapOffersReadOnlyAndCancelWithoutAdoptingItsBytes | kept | 0.02s; file/launch/input safety |
| 5 | 0.16 | testIdleRecoveryCoalescesEditsAndNeverAutosavesOriginalInPlace | kept | 0.16s; file/launch/input safety |

## EVCodeStylesTests
`src/mac/Editor/Tests/EVCodeStylesTests.swift` — 11 tests, 1.9 s; deleting 2

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.20 | testUnknownCodeThemeExtensionsPreserveContinuousEditUndo | kept | 0.20s; style inspector area (overlapping coverage) |
| 3 | 0.55 | testGlobalEditorHasIndependentTargetUndoAndPersistence | deleted | global code styles |
| 3 | 0.14 | testNamesAreExactAndDeletionAndRenamePersistWithoutResurrectingBuiltins | kept | 0.14s; style inspector area (overlapping coverage) |
| 4 | 0.02 | testPersistenceFailureRestoresPublishedDefinitionAndDoesNotAddUndo | kept | 0.02s; style inspector area (overlapping coverage) |
| 3 | 0.13 | testGlobalChangesReachTwoCodeBuffersWithoutDirtyingOrEnteringDocumentHistory | kept | 0.13s; style inspector area (overlapping coverage) |
| 4 | 0.04 | testMalformedSavedSheetRemainsReviewableAndExplicitRestoreRepairsIt | kept | 0.04s; style inspector area (overlapping coverage) |
| 2 | 0.45 | testGlobalMetricsChangesKeepViewportTextAnchoredAcrossTwoBuffers | deleted | anchor policy is core-tested |
| 3 | 0.15 | testExternalStyleReloadUpdatesAllBuffersAndKeepsLastValidStylesOnMalformedOrMissingFile | kept | 0.15s; style inspector area (overlapping coverage) |
| 4 | 0.05 | testLocalStyleWritesAreNotReimportedAndKeepSettingsUndo | kept | 0.05s; style inspector area (overlapping coverage) |
| 3 | 0.11 | testManualReloadRepublishesToEveryBufferEvenWhenTheFileLooksUnchanged | kept | 0.11s; style inspector area (overlapping coverage) |
| 4 | 0.03 | testManualReloadOfAnExternallyRemovedThemeUsesDefault | kept | 0.03s; style inspector area (overlapping coverage) |

## EVBaseParagraphStyleEditorTests
`src/mac/Editor/Tests/EVBaseParagraphStyleEditorTests.swift` — 4 tests, 1.8 s; deleting 3

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 1 | 0.62 | testBaseParagraphNextStyleIsFixedAcrossGeneratedAndNativeStyles | deleted | 0.62s; style inspector area (overlapping coverage) |
| 2 | 0.36 | testGlobalCodeBaseParagraphNextStyleIsSameStyle | deleted | 0.36s; style inspector area (overlapping coverage) |
| 3 | 0.43 | testEveryBaseParagraphOverrideIsLockedWhileValuesRemainEditable | kept | Base Paragraph lock |
| 2 | 0.42 | testGlobalCodeBaseParagraphOverrideClicksLeaveSettingsAndHistoryUnchanged | deleted | 0.42s; style inspector area (overlapping coverage) |

## EVCoreStateMenuIntegrationTests
`src/mac/Editor/Tests/EVCoreStateMenuIntegrationTests.swift` — 24 tests, 1.6 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testStandardNativeEditSelectorsReachCoreWhenDocumentViewIsFocused | kept | 0.06s; file/launch/input safety |
| 5 | 0.04 | testHistoryAvailabilityTitlesAndDirtyStateComeFromCore | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testBackendAcknowledgesOnlyTheExactCurrentSaveRevision | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testWordWrapUsesCoreViewStateIncludingExChanges | kept | 0.04s; file/launch/input safety |
| 5 | 0.05 | testLineEndingRadioActionIsAtomicUndoableAndShared | kept | 0.05s; file/launch/input safety |
| 5 | 0.13 | testEncodingMenuConversionsAreCheckedSharedAndExactlyUndoable | kept | 0.13s; file/launch/input safety |
| 5 | 0.04 | testRejectedLineEndingConversionChangesNeitherBytesStateNorHistory | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testFindActionsEnterCoreCommandLinesAndRepeatSearch | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testUseSelectionForFindUsesExactCoreSelectionAndNativeFindPasteboard | kept | 0.04s; file/launch/input safety |
| 5 | 0.06 | testJumpToSelectionRevealsCoreCursorAfterSelectionLeavesLayoutCoverage | kept | 0.06s; file/launch/input safety |
| 5 | 0.06 | testZoomIsExactViewLocalLayoutStateAndActualSizeRestoresOne | kept | 0.06s; file/launch/input safety |
| 5 | 0.19 | testDeleteUsesModeAppropriateCoreIntentions | kept | 0.19s; file/launch/input safety |
| 5 | 0.19 | testCaseTransformsAreVisualOnlyExceptNormalToggle | kept | 0.19s; file/launch/input safety |
| 5 | 0.05 | testSelectionCommandsKeepWholeLineAndTrailingNewlineSemantics | kept | 0.05s; file/launch/input safety |
| 5 | 0.04 | testSelectVisualRowUsesExactSoftWrappedRowEdges | kept | 0.04s; file/launch/input safety |
| 5 | 0.11 | testNativeCopyAndCutUpdateCoreRegistersBeforePasteboard | kept | 0.11s; file/launch/input safety |
| 5 | 0.06 | testNativeCopyUsesTheCoreSelectionWhenItsGeometryIsOffscreen | kept | 0.06s; file/launch/input safety |
| 5 | 0.04 | testNativePasteCanReplaceAVisualBlockUsingTheCoreRegisterPath | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testMarkdownBoldAndItalicMenusAreSourceBackedToggleableAndUndoable | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testSemanticStyleMenusApplyMixedFormattingWithoutChangingUnselectedText | kept | 0.04s; file/launch/input safety |
| 5 | 0.12 | testSemanticStyleMenusEnableMarkdownCaretAndDisablePlainTextAndVisualBlockSelections | kept | 0.12s; file/launch/input safety |
| 5 | 0.04 | testSemanticStyleMutationRejectsAChangedLogicalSelectionIdentity | kept | 0.04s; file/launch/input safety |
| 5 | 0.05 | testMarkdownSemanticStyleMenuUsesLogicalSelectionWhenGeometryIsOffscreen | kept | 0.05s; file/launch/input safety |
| 5 | 0.04 | testUnsupportedFormattingAndHostEffectItemsValidateDisabled | kept | 0.04s; file/launch/input safety |

## EVStyleEditorShortcutTests
`src/mac/Editor/Tests/EVStyleEditorShortcutTests.swift` — 2 tests, 1.6 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.17 | testNativeStyleSubmenusValidateWhenTheDocumentFormatChanges | kept |  |
| 4 | 1.44 | testBareF8OpensTheModelessStyleEditorInNormalAndInsertWithoutEditing | kept | F8 key routing |

## EVListDecorationIntegrationTests
`src/mac/Editor/Tests/EVListDecorationIntegrationTests.swift` — 8 tests, 1.6 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.17 | testMarkersExportAsNonTextFurnitureIncludingEmptyItemsAndSourceToggle | kept | 0.17s |
| 4 | 0.15 | testNativeTabAndShiftTabIndentInsideTheVisibleItem | kept | 0.15s |
| 3 | 0.30 | testBackspaceAtNestedItemStartUnindentsThenRemovesTheTopLevelMarker | kept | 0.30s |
| 4 | 0.15 | testEscapeKeepsTheNativeCaretOnTheTerminalEmptyParagraph | kept | 0.15s |
| 3 | 0.30 | testNativeArrowReturnToAnEmptyItemKeepsBackspaceAtItsVisibleStart | kept | 0.30s |
| 4 | 0.16 | testMarkerDrawingCullsOutsideDirtyRegionAtOrdinaryZoomStops | kept | 0.16s |
| 4 | 0.15 | testDecorationExportRejectsStaleIdentityAndDoesNotPartiallyCopy | kept | 0.15s |
| 4 | 0.18 | testNativeDrawingUsesMarkerParagraphColorWithoutMarkerSelection | kept | 0.18s |

## EVDefaultStyleIntegrationTests
`src/mac/Editor/Tests/EVDefaultStyleIntegrationTests.swift` — 6 tests, 1.5 s; deleting 2

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 1 | 0.63 | testMarkdownThemeRoundTripRestoresRenderedFontAndSurvivesResize | deleted | 0.63s; style inspector area (overlapping coverage) |
| 3 | 0.73 | testThemeStyleEditsUpdateExistingAndFutureDocumentsWithoutChangingSourceOrUndo | deleted | theme edits do not touch source |
| 3 | 0.11 | testDefaultThemeStyleEditsRemainInMemoryAndAreClonedByNewTheme | kept | 0.11s; style inspector area (overlapping coverage) |
| 4 | 0.03 | testMalformedSelectedThemeWarnsWithoutBlockingOrRewritingSource | kept | 0.03s; style inspector area (overlapping coverage) |
| 4 | 0.03 | testDefaultImportRejectsAnAlreadyOpenedViewWithoutMutation | kept | 0.03s; style inspector area (overlapping coverage) |
| 4 | 0.02 | testExportRejectsStaleSnapshotsAndNeverWritesPartialJSON | kept | 0.02s; style inspector area (overlapping coverage) |

## EVPageNavigationTests
`src/mac/Editor/Tests/EVPageNavigationTests.swift` — 2 tests, 1.5 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 0.87 | testRepeatedPhysicalPageKeysTraverseMarkdownAndCodeBeyondPartialLayouts | deleted | paging is core-tested |
| 4 | 0.60 | testRepeatedPageKeysPreserveInsertReplaceAndVisualModes | kept | only native page-key routing test |

## EVRichClipboardTests
`src/mac/Editor/Tests/EVRichClipboardTests.swift` — 8 tests, 1.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testMenuCopyAndExplicitPrimaryCopyExportMarkdownFormatting | kept | 0.06s |
| 4 | 0.54 | testPlatformCopyRetainsVimCharacterLineAndBlockSelectionsInBothDirections | kept | native copy |
| 5 | 0.09 | testPlatformCopyLeavesPendingMappingAndTemporaryVisualCommandUntouched | kept | 0.09s |
| 4 | 0.22 | testCodeCopyAndCopySourceExportOnlyPlainSource | kept | 0.22s |
| 4 | 0.18 | testSameFormatPastePreservesExactSourceAndUndoRedo | kept | 0.18s |
| 5 | 0.09 | testPasteAndMatchStyleIgnoresPrivateSourceAndRichFormatting | kept | 0.09s |
| 5 | 0.09 | testCodePastesRichClipboardAsExactPlainQuotesAndCopiesWithoutTypography | kept | 0.09s |
| 4 | 0.14 | testInvalidPrivateDataAllowsTypingAndFallsBackToPlainPaste | kept | 0.14s |

## EVStartupCommandTests
`src/mac/Editor/Tests/EVStartupCommandTests.swift` — 4 tests, 1.3 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testStartupYankMappingUsesNamedRegisterWithoutChangingSource | kept | 0.06s |
| 4 | 0.14 | testStartupSettingsAndMappingsApplyToEveryDocumentAndNewView | kept | 0.14s |
| 5 | 0.05 | testInvalidStartupLineIsReportedAndLaterSettingsStillApply | kept | 0.05s |
| 4 | 1.08 | testMappingTimeoutUsesTheShorterCompleteMapping | kept | only native mapping-timeout test |

## EVLayoutPaintIntegrationTests
`src/mac/Editor/Tests/EVLayoutPaintIntegrationTests.swift` — 7 tests, 1.3 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.04 | testScriptDecorationsFollowTheirOwnBaselineInAMixedRow | kept | 0.04s |
| 4 | 0.12 | testDefaultPaintUsesApplicationThemeAndSFProFourteen | kept | 0.12s |
| 5 | 0.04 | testMixedPaintRunRendersCanvasBackgroundForegroundAndDecorations | kept | 0.04s |
| 5 | 0.04 | testChangingThemeColorsRepaintsWithoutChangingSourceOrLayout | kept | 0.04s |
| 5 | 0.06 | testCaretInkLookupRemainsViewportBoundedInLargeDocument | kept | 0.06s |
| 3 | 0.69 | testLongFlowedParagraphDrawsOnlyVisibleInkAfterResizeAndMetricsInvalidation | deleted | bounded drawing |
| 4 | 0.28 | testEmptyDocumentCaretWidthUsesCurrentTypingFont | kept | 0.28s |

## EVBlockingEditorTests
`src/mac/BlockingTransportTests/EVBlockingEditorTests.swift` — 4 tests, 1.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.29 | testLaunchPreservesRelativeUnicodeSpacedAndOptionLikePathsThroughSymlink | kept | 0.29s; file/launch/input safety |
| 4 | 0.44 | testForwarderMayExitBeforeGUIConnects | kept | 0.44s; file/launch/input safety |
| 4 | 0.51 | testMissingGUIFailedLaunchAndSilentForwarderAreFailures | kept | 0.51s; file/launch/input safety |
| 5 | 0.00 | testRejectsDirectoryAndInvalidFileArgumentsBeforeLaunch | kept | 0.00s; file/launch/input safety |

## EVCharacterStyleMenuTests
`src/mac/Editor/Tests/EVCharacterStyleMenuTests.swift` — 6 tests, 1.2 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.35 | testBuiltinStyleAtNormalCaretEntersInsertAndRoundTripsTypedText | kept | 0.35s |
| 4 | 0.13 | testChoosingDefaultParagraphStopsPendingNamedStyle | kept | 0.13s |
| 2 | 0.48 | testCharacterStyleInheritsEachParagraphFontSizeAndClearsToItsParagraph | deleted | style resolution is core-tested |
| 5 | 0.10 | testDefaultParagraphClearsSelectedCharacterStylesAndUndoRestoresThem | kept | 0.10s |
| 5 | 0.10 | testBuiltinCharacterStyleAppliesToSelectedText | kept | 0.10s |
| 5 | 0.10 | testCharacterStyleSpansParagraphsAndLeavesOutsideTextUnassigned | kept | 0.10s |

## EVDocumentCloseReviewTests
`src/mac/AppShellTests/EVDocumentCloseReviewTests.swift` — 6 tests, 1.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.03 | testUntitledDeleteReviewsOnceWithoutSavingOrClearingTheCoreState | kept | 0.03s; file/launch/input safety |
| 5 | 0.10 | testCancelKeepsTheWindowAndASecondCloseReviewsAgain | kept | 0.10s; file/launch/input safety |
| 4 | 0.63 | testSaveDuringCloseWritesAndAcknowledgesBeforeClosingWithOneReview | kept | close-review save safety |
| 4 | 0.34 | testAnotherWindowKeepsDirtyDocumentAliveAndOnlyFinalWindowReviews | kept | 0.34s; file/launch/input safety |
| 5 | 0.04 | testTwoPanesOfTheSameDirtyDocumentReviewOnlyOnce | kept | 0.04s; file/launch/input safety |
| 5 | 0.09 | testDifferentDirtyPaneDocumentsEachReviewAndCancelPreservesBoth | kept | 0.09s; file/launch/input safety |

## EVDocumentExportTests
`src/mac/AppShellTests/EVDocumentExportTests.swift` — 5 tests, 1.2 s; deleting 2

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.55 | testExportPanelRestrictsHTMLAndSuggestsSafeCopyNames | deleted | export panel naming |
| 2 | 0.46 | testCancelDoesNotGenerateOrWriteAFile | deleted | simple cancellation; export tests remain |
| 4 | 0.20 | testExportWritesStyledCopyWithoutChangingDocumentOrSaveBaseline | kept | 0.20s |
| 5 | 0.01 | testExportRejectsCurrentSourceAliasesAndOtherOpenSources | kept | 0.01s |
| 5 | 0.00 | testFailedExportKeepsSourceAndDirtyState | kept | 0.00s |

## EVAccessibilityIntegrationTests
`src/mac/Editor/Tests/EVAccessibilityIntegrationTests.swift` — 5 tests, 1.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.20 | testCoreTextAndUnicodeRangesAreExposedInUTF16Coordinates | kept | 0.20s |
| 4 | 0.19 | testVisualLineAndBlockSelectionsExposeExactLogicalSegments | kept | 0.19s |
| 4 | 0.19 | testRangeAndPointGeometryUseExactLayoutAndSelectionExports | kept | 0.19s |
| 3 | 0.43 | testAccessibilitySelectionAndEditsRouteThroughCoreIntentions | kept | 0.43s |
| 4 | 0.20 | testStaleAndOutsideCoverageSelectionAndGeometryRequestsAreRejected | kept | 0.20s |

## EVMarkdownSwitchIntegrationTests
`src/mac/Editor/Tests/EVMarkdownSwitchIntegrationTests.swift` — 1 tests, 1.2 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 1.20 | testLargeMarkdownModeSwitchPublishesOneViewportAndPreservesSource | deleted | large switch is Rust-tested |

## EVWindowInputTests
`src/mac/Editor/Tests/EVWindowInputTests.swift` — 8 tests, 1.2 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.10 | testHeightUnitUsesBaseParagraphSpacingRatherThanMixedFontsAndIncludesZoom | kept | 0.10s |
| 5 | 0.05 | testCodeHeightUnitUsesTheCurrentCodeSheetAndViewZoom | kept | 0.05s |
| 4 | 0.14 | testCountedSplitAndNewPaneKeepTheirBufferAndSetEditorHeightInRows | kept | 0.14s |
| 5 | 0.10 | testWindowControlAliasesCountsAndCancelSurviveNativeRouting | kept | 0.10s |
| 4 | 0.16 | testVerticalExSplitsArrowFocusWidthCountsAndIndexedResizeKeepSource | kept | 0.16s |
| 2 | 0.48 | testMixedSplitStatusBarsKeepTheirHeightAndTextAlignmentAfterCollapseAndResize | deleted | status-bar cosmetics |
| 5 | 0.10 | testSplittingACollapsedPaneReportsNoRoomWithoutChangingTheBuffer | kept | 0.10s |
| 5 | 0.07 | testVisualBlockWindowPrefixKeepsSelectionAndColonOpensItsRange | kept | 0.07s |

## EVWhitespaceIntegrationTests
`src/mac/Editor/Tests/EVWhitespaceIntegrationTests.swift` — 12 tests, 1.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testWhitespaceAttributesRetainContributorScriptBaseline | kept | 0.00s |
| 5 | 0.05 | testExactMarkersAndExListTogglesPreserveSourceGeometryAndHistory | kept | 0.05s |
| 5 | 0.08 | testDefaultChangesPropagateToMultipleViewsAndRespectExplicitListOverride | kept | 0.08s |
| 5 | 0.10 | testWhitespaceMarkersAreSuppressedInMarkdownPreviewAndShownInCode | kept | 0.10s |
| 5 | 0.08 | testCodeWrappedIndentUpdatesBothViewsWithoutAddingTextOrMarginGeometry | kept | 0.08s |
| 5 | 0.05 | testCompositionMarkersUseTheOverlaySnapshotAndCancellationRetainsSource | kept | 0.05s |
| 4 | 0.10 | testNativeInvisibleCharactersMenuWorksInInsertModeAndFollowsExState | kept | 0.10s |
| 5 | 0.05 | testMalformedFFISettingsAreRejectedAtomically | kept | 0.05s |
| 5 | 0.05 | testMarkerFontOverridesFitOriginalSlotAndDoNotChangeLayout | kept | 0.05s |
| 4 | 0.22 | testClearedMarkerForegroundInheritsExactTextPaintAndRendersDecoration | kept | 0.22s |
| 4 | 0.26 | testMarkerInheritsNonFontRunAttributesAndExplicitOverridesPreserveGeometry | kept | 0.26s |
| 4 | 0.15 | testLongWhitespaceRowExportsOnlyViewportMarkersAndLookupUsesContainingSlot | kept | 0.15s |

## EVEditingReliabilityTests
`src/mac/Editor/Tests/EVEditingReliabilityTests.swift` — 9 tests, 1.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.09 | testIndentedMarkdownCodeEnterAndTypingRoundTrip | kept | 0.09s |
| 5 | 0.05 | testMarkdownFormattingMenusUseEffectiveStateAndPreserveSourceOnUndo | kept | 0.05s |
| 3 | 0.34 | testNativeMultilingualInputAcrossFormatsSurvivesResizeAndMetricsChanges | kept | 0.34s |
| 4 | 0.15 | testFontInvalidationRefreshesHitTestingWithoutReplayingText | kept | 0.15s |
| 5 | 0.08 | testMetricRecoveryInLargeDocumentKeepsVisibleExportBounded | kept | 0.08s |
| 4 | 0.11 | testBlankCanvasHitTestsDocumentEdgesWithoutGuessingUnmaterializedContent | kept | 0.11s |
| 5 | 0.09 | testMarkdownPreviewToggleAndEncodingMenuPreserveSourceAndHistory | kept | 0.09s |
| 4 | 0.23 | testMarkdownPreviewToggleKeepsInteriorUnicodeCursorAtTheSameRepeatedOccurrence | kept | 0.23s |
| 5 | 0.04 | testNativePrintableKeysMoveAndEditVisualBlockUsingLayout | kept | 0.04s |

## EVImageScrollingTests
`src/mac/Editor/Tests/EVImageScrollingTests.swift` — 1 tests, 1.2 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 1.16 | testScrollingThroughDemoUsesTextSizedWheelStepsOnLoadedImages | deleted | demo-sized scroll sweep |

## EVExternalFileChangeTests
`src/mac/AppShellTests/EVExternalFileChangeTests.swift` — 9 tests, 1.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testReplacementDeletionAndSameLengthChangesNeverReloadDirtyBuffer | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testMetadataPreservingChangeIsDetectedAndStaleCheckCannotReplaceNewBaseline | kept | 0.01s; file/launch/input safety |
| 5 | 0.09 | testActivationAndChecktimeUseDialogWithoutStatusWarnings | kept | 0.09s; file/launch/input safety |
| 5 | 0.06 | testNativeSaveSaveAsAndSaveToRequireOneChoiceAndKeepDirtyOnCancel | kept | 0.06s; file/launch/input safety |
| 4 | 0.89 | testAllExSaveRoutesIncludingForceCancelWithoutWritingOrClosing | kept | external-change safety on every save route |
| 5 | 0.03 | testForcedRangedWriteQuitStillAsksAndDoesNotCloseOnCancel | kept | 0.03s; file/launch/input safety |
| 5 | 0.01 | testRetargetedSymbolicLinkStillChecksTheLoadedFingerprint | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testNativeWriterRechecksAfterSnapshotPreparation | kept | 0.01s; file/launch/input safety |
| 5 | 0.03 | testDeletedFileAndFurtherQueuedChangesRequireDistinctChoices | kept | 0.03s; file/launch/input safety |

## EVListParagraphTests
`src/mac/Editor/Tests/EVListParagraphTests.swift` — 6 tests, 1.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.41 | testPhysicalOptionReturnAndKeypadEnterSplitWithoutAddingAListMarker | kept | 0.41s |
| 5 | 0.06 | testReturnAndShiftReturnKeepTheirMeaningsInsideTheContinuationParagraph | kept | 0.06s |
| 4 | 0.12 | testMenuAndKeyEquivalentEnterInsertFromNormalAndRespectNativeFieldFocus | kept | 0.12s |
| 5 | 0.06 | testKeyEquivalentRetainsCoreMappingsAndLiteralNext | kept | 0.06s |
| 4 | 0.27 | testParagraphMenuDisablesUnsupportedContextsAndSelections | kept | 0.27s |
| 4 | 0.22 | testParagraphMenuDoesNotConsumePendingCommandsOrComposition | kept | 0.22s |

## EVExternalFileReviewTests
`src/mac/AppShellTests/EVExternalFileReviewTests.swift` — 15 tests, 1.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testKeepBufferDeduplicatesOneDiskStateButPreservesOverwriteProtection | kept | 0.01s; file/launch/input safety |
| 5 | 0.02 | testLoadFileRequiresChoiceAndRefreshesSharedBufferAndBaseline | kept | 0.02s; file/launch/input safety |
| 5 | 0.01 | testReturningToBaselineAllowsSameExternalContentsToPromptAgain | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testReloadPreservesCodeAndSourceViewFormats | kept | 0.01s; file/launch/input safety |
| 5 | 0.09 | testReloadKeepsWatchingSelectedSymlinkAfterCanonicalBaselineReceipt | kept | 0.09s; file/launch/input safety |
| 5 | 0.01 | testReturnToBaselineWhilePromptIsOpenDoesNotAcknowledgeObsoleteChange | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testReloadInstallsOnlyVerifiedBytesWhenPathIsReplacedAfterRead | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testDeletionKeepsBufferAndRecreationOffersAnotherChoice | kept | 0.01s; file/launch/input safety |
| 5 | 0.08 | testMonitorPromptsForAtomicReplacementWithoutActivationOrChecktime | kept | 0.08s; file/launch/input safety |
| 4 | 0.37 | testSaveDoesNotPromptForItsOwnAtomicReplacement | kept | 0.37s; file/launch/input safety |
| 4 | 0.39 | testSaveToBoundPathUpdatesWatchBaselineWithoutClearingDirtyState | kept | 0.39s; file/launch/input safety |
| 5 | 0.01 | testOnePendingReviewAcrossConcurrentChecksAndNewerEditsNeedAnotherChoice | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testDiskChangingDuringPromptIsReviewedAgainBeforeLoading | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testRetargetAndCloseInvalidatePendingConsentAndStopOldWatch | kept | 0.01s; file/launch/input safety |
| 5 | 0.01 | testFailedReloadPreservesBufferAndCanBeRetried | kept | 0.01s; file/launch/input safety |

## EVHostEffectsIntegrationTests
`src/mac/Editor/Tests/EVHostEffectsIntegrationTests.swift` — 16 tests, 1.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.04 | testClipboardAndPrimarySnapshotsStayDistinctWhileSharingNativeStorage | kept | 0.04s; file/launch/input safety |
| 5 | 0.15 | testDefaultPasteboardAdapterSurvivesSequentialSurfaceAutoreleasePools | kept | 0.15s; file/launch/input safety |
| 5 | 0.08 | testSystemClipboardYankAndPutUseCoreRegisterSemantics | kept | 0.08s; file/launch/input safety |
| 5 | 0.04 | testPrimaryRegisterEffectRetainsTargetShapeAndExactPostTurnIdentity | kept | 0.04s; file/launch/input safety |
| 5 | 0.08 | testNativeCharacterPasteInsertsBeforeCaretAndLeavesItAfterInsertedText | kept | 0.08s; file/launch/input safety |
| 5 | 0.16 | testNativeLinewisePasteStartsAboveCurrentLineIncludingBlankCommitMessageLine | kept | 0.16s; file/launch/input safety |
| 5 | 0.04 | testExplicitClipboardPutStillUsesVimPutAfterPlacement | kept | 0.04s; file/launch/input safety |
| 5 | 0.08 | testNativePasteCancelsPendingCommandAndLiteralNextInterpretation | kept | 0.08s; file/launch/input safety |
| 5 | 0.04 | testClipboardWriteFailureStillReleasesOwnedBatch | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testTypedOptionsPrintAndSubstitutionResultsReachMessageSurface | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testResolvedMarksRegistersAndJumpsRenderWithoutFrontendProjectionReads | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testFileRequestsPreserveKindFlagsPathRangeAndExactIdentity | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testArgumentRequestsPreserveCountCaseWritePathAndInitialLine | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testUppercaseZZUsesExactSaveQuitRequestAndReportsSaveFailure | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testHostFailureIsRenderedAndWriteQuitRemainsOneTypedRequest | kept | 0.04s; file/launch/input safety |
| 5 | 0.04 | testStaleAndInvalidBatchesAreRejectedBeforePasteboardMutation | kept | 0.04s; file/launch/input safety |

## EVPointerDrawingPerformanceTests
`src/mac/Editor/Tests/EVPointerDrawingPerformanceTests.swift` — 5 tests, 1.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.18 | testHoverIsAHollowTwentyPercentOverlayAndPartialDamageMatchesFreshDrawing | kept | 0.18s |
| 4 | 0.17 | testVisiblePointerSelectionUsesOneRefreshAndBoundedDamageInLargeDocument | kept | 0.17s |
| 3 | 0.42 | testPartialSelectionFramesMatchFreshRenderingAndProfileNativeDrawing | kept | 0.42s |
| 4 | 0.17 | testContentAndEnvironmentChangesInvalidateDamageAndMatchFreshRendering | kept | 0.17s |
| 5 | 0.05 | testPointerRefreshesExternallyChangedGeometryBeforeHitTesting | kept | 0.05s |

## EVNativeSelectionTests
`src/mac/Editor/Tests/EVNativeSelectionTests.swift` — 13 tests, 1.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testShiftArrowsReplaceWholeGraphemeAndUndoOnce | kept | 0.06s; file/launch/input safety |
| 5 | 0.06 | testSelectorsAndCommandShiftArrowExtendToInsertionBoundaries | kept | 0.06s; file/launch/input safety |
| 5 | 0.10 | testUnshiftedArrowsCollapseSelectionLikeAppKit | kept | 0.10s; file/launch/input safety |
| 5 | 0.07 | testOptionArrowsNavigateParagraphsAcrossWrappedRowsAndExtendSelection | kept | 0.07s; file/launch/input safety |
| 5 | 0.12 | testAccessibilityRangesKeepExactEndsInSelectAndVisualModes | kept | 0.12s; file/launch/input safety |
| 5 | 0.06 | testSelectCompositionIsOverlayUntilCommitAndCancelKeepsSelection | kept | 0.06s; file/launch/input safety |
| 5 | 0.06 | testNativeCopyRetainsSelectionThenTypingReplacesIt | kept | 0.06s; file/launch/input safety |
| 5 | 0.06 | testNativeBlockCopyRetainsExactSegments | kept | 0.06s; file/launch/input safety |
| 5 | 0.06 | testAlternateOptionsAndCommandSelectionsAreIndependent | kept | 0.06s; file/launch/input safety |
| 5 | 0.14 | testSelectionOptionsAreGlobalAcrossExistingAndNewDocumentsWithoutChangingModes | kept | 0.14s; file/launch/input safety |
| 5 | 0.05 | testStatusDistinguishesNativeSelectionFromVimVisualAndSelect | kept | 0.05s; file/launch/input safety |
| 5 | 0.05 | testNoAutoselectUsesVimInclusiveSelectionAndDeleteReturnsNormal | kept | 0.05s; file/launch/input safety |
| 5 | 0.11 | testNativeMenuSelectionPreservesInsertionBoundaryAndResumeMode | kept | 0.11s; file/launch/input safety |

## EVDocumentWindowControllerTests
`src/mac/AppShellTests/EVDocumentWindowControllerTests.swift` — 21 tests, 1.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testFileURLChangesRefreshEveryStatusPathWithoutSurfaceUpdates | kept | 0.06s; file/launch/input safety |
| 5 | 0.03 | testPendingInteractionRevealsHiddenStatusBarUntilFinished | kept | 0.03s; file/launch/input safety |
| 5 | 0.03 | testInitialShowUsesRequestedContentSizeAndFillsCanvasAboveStatusBar | kept | 0.03s; file/launch/input safety |
| 5 | 0.03 | testInitialShowOverridesPreShowGeometryWithoutRestorationOrTabbing | kept | 0.03s; file/launch/input safety |
| 5 | 0.03 | testLaterShowsPreserveAUserResize | kept | 0.03s; file/launch/input safety |
| 5 | 0.03 | testInitialShowRestoresStoredFrameAndRecordsLaterResize | kept | 0.03s; file/launch/input safety |
| 5 | 0.03 | testStatusBarMenuValidationToggleAndLayoutStayWindowLocal | kept | 0.03s; file/launch/input safety |
| 5 | 0.03 | testNewWindowCommandRoutesToDocumentWhileOtherCommandsStaySurfaceOwned | kept | 0.03s; file/launch/input safety |
| 5 | 0.03 | testStackedSplitSharesBackendAndClosingOnePaneKeepsTheDocument | kept | 0.03s; file/launch/input safety |
| 5 | 0.00 | testDocumentIdentityRecognizesSymbolicAndHardLinks | kept | 0.00s; file/launch/input safety |
| 5 | 0.03 | testSplitReusesAnotherOpenDocumentAndRebindsWhenTheOriginalPaneCloses | kept | 0.03s; file/launch/input safety |
| 5 | 0.04 | testLocalLinkReusesExistingDocumentAndItsUnsavedContentWithoutBackingFile | kept | 0.04s; file/launch/input safety |
| 5 | 0.03 | testLocalLinkOpensUnopenedDocumentInNewWindow | kept | 0.03s; file/launch/input safety |
| 5 | 0.07 | testLocalLinkFocusesExistingInactivePaneInSameOrOtherWindow | kept | 0.07s; file/launch/input safety |
| 5 | 0.03 | testLocalLinkPrefersExistingActivePaneAmongMultipleViews | kept | 0.03s; file/launch/input safety |
| 5 | 0.04 | testLocalLinkReusesViewOpenedWhileItsLoadWasPending | kept | 0.04s; file/launch/input safety |
| 5 | 0.03 | testClosingSourceWindowCancelsPendingLocalLinkWindow | kept | 0.03s; file/launch/input safety |
| 5 | 0.01 | testLocalLinkFailureAndForeignSurfaceCannotReplaceTheSource | kept | 0.01s; file/launch/input safety |
| 5 | 0.26 | testFileDropReplacesExactCleanPaneAndOpensRemainingFilesInNewWindows | kept | 0.26s; file/launch/input safety |
| 5 | 0.05 | testFileDropOnDirtyPaneOpensNewWindowWithTheExistingBackend | kept | 0.05s; file/launch/input safety |
| 5 | 0.05 | testFileDropRechecksDirtyStateAfterAsynchronousOpenAndFailurePreservesPane | kept | 0.05s; file/launch/input safety |

## EVStyleMenuBridgeTests
`src/mac/Editor/Tests/EVStyleMenuBridgeTests.swift` — 6 tests, 0.9 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.05 | testParagraphShortcutValidationRetainsCurrentStyleCheckmark | kept | 0.05s; style inspector area (overlapping coverage) |
| 4 | 0.05 | testListIndentMenuValidatesAndDispatchesFromTheMiddleOfAnItem | kept | 0.05s; style inspector area (overlapping coverage) |
| 4 | 0.05 | testHeadingMenuWritesFormatMarkersAndRejectsStaleActions | kept | 0.05s; style inspector area (overlapping coverage) |
| 4 | 0.04 | testLiveCoreCatalogueExposesOnlyBasePlainTextStylesAndDisablesAssignment | kept | 0.04s; style inspector area (overlapping coverage) |
| 3 | 0.71 | testEditActionRetargetsSingletonToExactStableStyleIdentity | deleted | menu edit action retargets inspector |
| 4 | 0.04 | testAssignmentPayloadCannotBypassDisabledAdapterCapability | kept | 0.04s; style inspector area (overlapping coverage) |

## EVFontFamilyApplicationTests
`src/mac/Editor/Tests/EVFontFamilyApplicationTests.swift` — 1 tests, 0.9 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 0.94 | testNativeFamilyChoiceUpdatesCodePreviewGlyphsHistoryAndPersistedSettings | deleted | overlaps compact-control family tests |

## EVParagraphEditingKeyTests
`src/mac/Editor/Tests/EVParagraphEditingKeyTests.swift` — 3 tests, 0.9 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.54 | testPhysicalShiftReturnAndLineBreakSelectorKeepOneParagraphWhileReturnCreatesTwo | kept | native Return routing |
| 4 | 0.26 | testNativeBackspaceResetsListsAndJoinsQuotesWhileEmptyQuoteReturnResetsStyle | kept | 0.26s |
| 4 | 0.14 | testNativeOpenAboveAndBelowUseTheHeadingFollowingStyle | kept | 0.14s |

## EVIncrementalMatchStyleTests
`src/mac/Editor/Tests/EVIncrementalMatchStyleTests.swift` — 1 tests, 0.9 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 0.93 | testInternalStyleIsEditableButAbsentFromApplicationMenusForEveryFormat | deleted | slow inspector check of one internal style |

## EVStatusLineTests
`src/mac/AppShellTests/EVStatusLineTests.swift` — 15 tests, 0.9 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testStatusInsetsAndHeightUseTheStyleBox | kept | 0.00s |
| 4 | 0.15 | testThickBordersCenterTextInsidePaddingAndClipOuterEdges | kept | 0.15s |
| 4 | 0.20 | testOversizedCommandFontClipsInsideBarWithoutChangingContainerHeight | kept | 0.20s |
| 5 | 0.00 | testCaretPositionIsRightAlignedAndEverythingElseIsLeftAligned | kept | 0.00s |
| 5 | 0.01 | testResizeHoverCoversOnlyEmptyMovableBackgroundAndFollowsPresentation | kept | 0.01s |
| 5 | 0.07 | testStatusPixelsAfterNarrowingAndWideningMatchFreshLayout | kept | 0.07s |
| 5 | 0.01 | testACommandLineReplacesTheLeftGroupAndKeepsTheCaretWidget | kept | 0.01s |
| 5 | 0.01 | testTheCommandCaretTracksItsOffsetAndOnlyBlinksWhenActive | kept | 0.01s |
| 5 | 0.01 | testClickingTheCommandAreaReportsTheNearestUTF8Offset | kept | 0.01s |
| 5 | 0.00 | testMultibyteCommandTextMeasuresByUTF8Offsets | kept | 0.00s |
| 4 | 0.22 | testCommandCaretUsesFullProportionalShapingAndUnicodeScalarPositions | kept | 0.22s |
| 5 | 0.00 | testOrdinaryStatusAndCommandPromptsDoNotConstructOutputControls | kept | 0.00s |
| 4 | 0.22 | testDeferredOutputUsesCurrentThemeAndPreservesSelectionAcrossStatusUpdates | kept | 0.22s |
| 5 | 0.01 | testOutputUsesNormalColorsWithLeftCloseAndNeverCoversPosition | kept | 0.01s |
| 5 | 0.01 | testCommandPromptTakesPriorityOverOutput | kept | 0.01s |

## EVSingleInstanceTests
`src/mac/AppShellTests/EVSingleInstanceTests.swift` — 7 tests, 0.9 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.11 | testLaunchWaitsForHandlerAndPreservesArgumentsAndWorkingDirectory | kept | 0.11s; file/launch/input safety |
| 5 | 0.03 | testHandlerDiagnosticIsAcknowledgedWithoutStartingAnotherInstance | kept | 0.03s; file/launch/input safety |
| 5 | 0.03 | testSimultaneousLaunchesElectOnePrimaryAndDeliverEveryOtherLaunchOnce | kept | 0.03s; file/launch/input safety |
| 5 | 0.22 | testCrossProcessStartupRaceHandoffAndStaleSocketRecovery | kept | 0.22s; file/launch/input safety |
| 5 | 0.07 | testMalformedFrameIsRejectedAndListenerStillAcceptsValidLaunch | kept | 0.07s; file/launch/input safety |
| 5 | 0.00 | testRejectsSharedOrSymlinkedInstanceDirectory | kept | 0.00s; file/launch/input safety |
| 4 | 0.46 | testUnacknowledgedRequestIsNeverRetriedOrPromotedToPrimary | kept | single-instance safety |

## EVCodeStyleInheritanceTests
`src/mac/Editor/Tests/EVCodeStyleInheritanceTests.swift` — 3 tests, 0.9 s; deleting 2

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.03 | testSavedCanonicalStylesPreserveInheritanceWithoutRewritingTheFile | kept | 0.03s; style inspector area (overlapping coverage) |
| 2 | 0.52 | testCommentParentEditsReachRustCapturesAndChildOverridesSurviveHistoryAndReload | deleted | inheritance covered by faster sibling |
| 2 | 0.31 | testBuiltinCodeDeclarationCanBeUncheckedAndStaysClearedAfterReload | deleted | 0.31s; style inspector area (overlapping coverage) |

## EVSearchHighlightTests
`src/mac/Editor/Tests/EVSearchHighlightTests.swift` — 10 tests, 0.8 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.05 | testSubmittedSearchPublishesExactPaintWithoutChangingDocumentHistory | kept | 0.05s |
| 5 | 0.05 | testSearchOptionQueriesUseNativeOptionNames | kept | 0.05s |
| 5 | 0.05 | testIncrementalSearchDefaultsToOnWithoutStartupOverride | kept | 0.05s |
| 5 | 0.10 | testIncrementalSearchPaintsEveryCurrentMatchWithoutPersistentHighlights | kept | 0.10s |
| 5 | 0.10 | testIncrementalSearchRestoresPersistentPatternOnCancelAndCommitsItOnAccept | kept | 0.10s |
| 4 | 0.10 | testIncrementalSearchTemporarilyOverridesHighlightSuppression | kept | 0.10s |
| 5 | 0.07 | testSharedSearchOptionsRefreshInactivePaneWithoutMovingItsCursor | kept | 0.07s |
| 5 | 0.10 | testIncrementalPreviewRevealsMatchAndEscapeRestoresScrolledViewport | kept | 0.10s |
| 5 | 0.07 | testDetachingRetiresPendingNativeSearchPolling | kept | 0.07s |
| 4 | 0.16 | testRunLoopFinishesSearchWithoutAdditionalInput | kept | 0.16s |

## EVControlKeyTests
`src/mac/Editor/Tests/EVControlKeyTests.swift` — 2 tests, 0.8 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.78 | testControlEditingAliasesMatchTheirPhysicalKeysInInsertAndReplace | kept | native control-key routing |
| 5 | 0.05 | testControlWordArrowsUsePortableWordMovementAndRemainQuotable | kept | 0.05s; file/launch/input safety |

## CoreTextImageResourcesTests
`src/mac/CoreTextProvider/Tests/CoreTextImageResourcesTests.swift` — 15 tests, 0.8 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testRemoteAndExecutableLocationsNeverStartLocalLoads | kept | 0.00s |
| 5 | 0.00 | testRelativePathsPreserveEscapesAndOnlyUseTheDocumentDirectory | kept | 0.00s |
| 5 | 0.01 | testPassiveRasterDecodeUsesPixelDimensionsAndRejectsSVG | kept | 0.01s |
| 4 | 0.14 | testPreviewFileLimitUsesDecimalMegabytesAndIncludesExactBoundary | kept | 0.14s |
| 5 | 0.01 | testPixelLimitsInclude5000AndReject5001InEitherDimension | kept | 0.01s |
| 5 | 0.04 | testLimitedImagesRemainStableUntilReloadAndRecoverWhenResourceFits | kept | 0.04s |
| 5 | 0.02 | testLimitKnowledgeResetsOnDocumentIdentityChangeAndCorruptionStaysBroken | kept | 0.02s |
| 5 | 0.01 | testFullQueueBackpressureRetriesVisibleLoadsAfterFailures | kept | 0.01s |
| 5 | 0.01 | testImageDrivenMembershipChangesDoNotResetAttemptBudget | kept | 0.01s |
| 5 | 0.02 | testBudgetKeepsVisibleImagesAndDeclinesWithoutReloadChurn | kept | 0.02s |
| 5 | 0.07 | testEntryEvictionKeepsVisibleEntriesAndTracksItsChosenKey | kept | 0.07s |
| 5 | 0.02 | testDocumentURLChangeDrainsOldJobsWithoutCancellingNewEpoch | kept | 0.02s |
| 3 | 0.42 | testKnownIntrinsicSizeSurvivesBitmapEvictionSynchronously | kept | 0.42s |
| 5 | 0.03 | testReloadReadsChangedDiskFileAndCanRecoverBrokenImages | kept | 0.03s |
| 5 | 0.00 | testMetadataBudgetExhaustionStopsAdmissionRefreshRetries | kept | 0.00s |

## EVParagraphStyleMenuTests
`src/mac/Editor/Tests/EVParagraphStyleMenuTests.swift` — 5 tests, 0.8 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.27 | testInternalListFamiliesRemainEditableAndOnlyCurrentStyleAppearsInMenu | kept | 0.27s |
| 4 | 0.18 | testListIndentCommandsFollowCoreCapabilitiesAndRoundTripExactSource | kept | 0.18s |
| 4 | 0.13 | testBlockQuoteMenuAssignmentPersistsAndUndoRedoRestoreExactSource | kept | 0.13s |
| 4 | 0.10 | testBlockQuoteMenuAtDocumentEndCreatesBlankQuoteReadyForTyping | kept | 0.10s |
| 5 | 0.09 | testBlockQuoteBorderIsNonTextFurnitureAndCullsOutsideDirtyRegion | kept | 0.09s |

## EVProseQualityTests
`src/mac/Editor/Tests/EVProseQualityTests.swift` — 3 tests, 0.7 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 2 | 0.46 | testCommonEditingCommandsDoNotPoisonReflowOrUndo | deleted | reflow/undo are core-tested |
| 4 | 0.19 | testRepeatedProseFormattingUndoAndReflowAcrossRichFormats | kept | 0.19s |
| 5 | 0.09 | testEndOfMarkdownRepeatedReturnsRemainEditable | kept | 0.09s |

## EVCodeOpeningTests
`src/mac/Editor/Tests/EVCodeOpeningTests.swift` — 11 tests, 0.7 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.04 | testProtocolReadPreservesFilenameForAutomaticCodeAndRustPaint | kept | 0.04s |
| 5 | 0.05 | testDocumentURLReadDetectsRustAndPublishesSyntaxPaint | kept | 0.05s |
| 4 | 0.10 | testMarkdownOpensAsCodeWithPreviewOffAndHTMLOpensAsCode | kept | 0.10s |
| 5 | 0.03 | testNativeHTMLDataAndLegacyRecoveryOpenAsCode | kept | 0.03s |
| 4 | 0.13 | testUnknownDocumentURLTypesOpenAsTextWithoutChangingSourceBytes | kept | 0.13s |
| 5 | 0.03 | testGenericDataDocumentTypeStillDetectsCodeFromFilename | kept | 0.03s |
| 4 | 0.12 | testFilenameAndLoadMarkersSelectCodeBeforeViewsExistAndKeepSourceClean | kept | 0.12s |
| 5 | 0.06 | testExplicitFormatsAndExistingRichOpeningDefaultsWinOverAutomaticCodeSelection | kept | 0.06s |
| 5 | 0.04 | testConfiguredFilenameAssociationsApplyBeforeFirstDetectionAndRemainLiteral | kept | 0.04s |
| 5 | 0.08 | testDiagnosticsRefreshWithoutAChangedPresentation | kept | 0.08s |
| 5 | 0.05 | testCodeDisablesDocumentFormattingAndRetainsGlobalStyleEditing | kept | 0.05s |

## EVPendingTypingStyleTests
`src/mac/Editor/Tests/EVPendingTypingStyleTests.swift` — 5 tests, 0.7 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.20 | testCommandFormattingShortcutsWorkWithoutMenuItemsAndRespectFocus | kept | 0.20s |
| 4 | 0.19 | testFormattingAcceleratorsLeavePendingCommandsAndCompositionUntouched | kept | 0.19s |
| 4 | 0.18 | testUnavailableFormattingAcceleratorsDoNotMutateSourceOrInvokeHeadings | kept | 0.18s |
| 5 | 0.05 | testFormattingMenuCannotFallThroughANativeFieldInAnotherWindow | kept | 0.05s |
| 5 | 0.04 | testOptionItalicKeepsSourceCleanUntilTypedContentAndUndoIsExact | kept | 0.04s |

## EVHTMLExportIntegrationTests
`src/mac/Editor/Tests/EVHTMLExportIntegrationTests.swift` — 2 tests, 0.6 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.28 | testCodeExportsEscapedLiteralTextWithSyntaxStyles | kept | 0.28s |
| 3 | 0.35 | testNativeExportCommandUsesTheActivePaneAndHTMLPanel | kept | 0.35s |

## EVDocumentScrollbarsTests
`src/mac/Editor/Tests/EVDocumentScrollbarsTests.swift` — 8 tests, 0.6 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testLegacyUsesNativeControlsAndStableGuttersWhileHorizontalAvailabilityFades | kept | 0.00s |
| 5 | 0.00 | testOverlayActivityFadesAndKeyboardViewportChangesRevealItAgain | kept | 0.00s |
| 5 | 0.00 | testNativePageLineAndKnobActionsRequestCheckedNormalizedCorePositions | kept | 0.00s |
| 5 | 0.00 | testKnobTrackingPreventsHideAndReexportsDoNotJumpTheNativeThumb | kept | 0.00s |
| 5 | 0.00 | testPreferredStyleNotificationChangesNativeStyleAndGeometryOnce | kept | 0.00s |
| 5 | 0.00 | testVisibilityTimerStopsWhenDetachedAndDoesNotRetainTheComponent | kept | 0.00s |
| 4 | 0.25 | testKnobsPaintVisibleInkInBothOrientationsOnLightAndDarkCanvases | kept | 0.25s |
| 3 | 0.38 | testPaintedKnobMidpointsRouteToNativeControlsInBothOrientations | kept | 0.38s |

## EVWordCompletionTests
`src/mac/Editor/Tests/EVWordCompletionTests.swift` — 9 tests, 0.6 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.07 | testControlKeysPublishNativePopupAndEscapeAcceptsThenLeavesInsert | kept | 0.07s |
| 5 | 0.06 | testTypingAcceptsBackendSelectedCandidateAndRunsOrdinaryInput | kept | 0.06s |
| 5 | 0.06 | testNativePopupDisplaysSuppliedSelectionWithoutTakingKeyboardFocus | kept | 0.06s |
| 5 | 0.06 | testNativeMarkedTextAcceptsCompletionBeforeStartingComposition | kept | 0.06s |
| 5 | 0.06 | testWindowFocusLossAcceptsCompletionAndHidesPopup | kept | 0.06s |
| 5 | 0.07 | testNativeExplicitReplacementAndCommandShortcutAcceptBeforeTheirNormalEffect | kept | 0.07s |
| 4 | 0.10 | testLargeDocumentSearchingYieldsAndEscapeStopsNativePolling | kept | 0.10s |
| 4 | 0.14 | testBackendWordAnchorKeepsNativePopupStationaryAcrossDifferentCandidateLengths | kept | 0.14s |
| 5 | 0.00 | testNativePopupStaysOnScreenAndMovesAboveCaretNearBottom | kept | 0.00s |

## EVNativeFileDragIntegrationTests
`src/mac/Editor/Tests/EVNativeFileDragIntegrationTests.swift` — 1 tests, 0.6 s; deleting 1

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 3 | 0.59 | testActualDragCallbacksReplaceCleanDocumentThenKeepDirtyTextInItsWindow | deleted | 0.59s |

## EVCoreFrontendIntegrationTests
`src/mac/Editor/Tests/EVCoreFrontendIntegrationTests.swift` — 11 tests, 0.6 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.04 | testEditingLayoutRenderingAndNativeHistoryRoundTrip | kept | 0.04s |
| 5 | 0.04 | testPointerHitTestAndShiftExtensionRemainCoreOwned | kept | 0.04s |
| 5 | 0.04 | testIMECommitIsAtomicAndCancelLeavesSourceUntouched | kept | 0.04s |
| 5 | 0.00 | testFrontendUsesSFProFourteenPointDefault | kept | 0.00s |
| 4 | 0.11 | testRetainedEditorViewBecomesInertAfterSurfaceControllerRelease | kept | 0.11s |
| 5 | 0.06 | testDocumentPaddingUsesCoreViewportAfterNativeScrollbarGutters | kept | 0.06s |
| 5 | 0.05 | testKeyboardViewportOriginIsReflectedByViewCoordinates | kept | 0.05s |
| 5 | 0.06 | testHorizontalViewportOriginRoundTripsThroughCore | kept | 0.06s |
| 5 | 0.06 | testIdentityBoundVerticalViewportUpdatesBoundedRegionalCoverage | kept | 0.06s |
| 5 | 0.05 | testSharedDocumentChangeRefreshesEverySurfaceExactlyOnce | kept | 0.05s |
| 5 | 0.06 | testSharedCommitClearsInvalidatedSiblingMarkedText | kept | 0.06s |

## EVDocumentScrollbarsIntegrationTests
`src/mac/Editor/Tests/EVDocumentScrollbarsIntegrationTests.swift` — 7 tests, 0.6 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.07 | testHorizontalScrollerUsesOnlyVisibleLinesAndResetsWhenTheyLeave | kept | 0.07s |
| 5 | 0.04 | testWrappedUnbreakableLineKeepsHorizontalScrollerAvailable | kept | 0.04s |
| 4 | 0.12 | testOverflowPaintContinuesUnderVerticalScrollerWithoutChangingWrapWidth | kept | 0.12s |
| 5 | 0.06 | testVerticalThumbAndPageActionsScrollWithoutMovingCaretOrEditing | kept | 0.06s |
| 5 | 0.06 | testScrollbarRefreshKeepsLargeDocumentExportsBounded | kept | 0.06s |
| 4 | 0.13 | testMarkdownPageKeysAndScrollbarShareStablePaddedEndpoints | kept | 0.13s |
| 5 | 0.09 | testNativeScrollbarAppearanceFollowsTheCanvasForContrast | kept | 0.09s |

## EVBlockingEditTransportTests
`src/mac/BlockingTransportTests/EVBlockingEditTransportTests.swift` — 8 tests, 0.6 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testCompletionForwardsExactStatusAndFinishesOnlyOnce | kept | 0.00s |
| 5 | 0.00 | testDroppingUnacceptedCompletionReportsFailure | kept | 0.00s |
| 4 | 0.16 | testEditingDoesNotInheritStartupTimeout | kept | 0.16s |
| 4 | 0.13 | testStartupTimeoutAndFailedLaunchAreFailures | kept | 0.13s |
| 4 | 0.15 | testCrashAndPartialStatusNeverReportSuccessfulEditing | kept | 0.15s |
| 5 | 0.00 | testEndpointsArePrivateAndRemovedAfterUse | kept | 0.00s |
| 5 | 0.00 | testRejectsMalformedSharedAndSymlinkedCompletionEndpoints | kept | 0.00s |
| 4 | 0.11 | testDeadCallerCannotRaiseSIGPIPEInGUI | kept | 0.11s |

## EVMarkdownViewPreferenceTests
`src/mac/Editor/Tests/EVMarkdownViewPreferenceTests.swift` — 5 tests, 0.6 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.15 | testNativeMarkdownOpensUseRememberedChoiceAcrossConfigurationReloads | kept | 0.15s |
| 4 | 0.23 | testToggleAndHistoryRememberActualTransitionsWithoutBackgroundViewsOverwritingThem | kept | 0.23s |
| 5 | 0.06 | testExplicitRecoveryAndReloadKeepTheirViewWithoutChangingPreference | kept | 0.06s |
| 5 | 0.05 | testRejectedSwitchDoesNotRememberRequestedView | kept | 0.05s |
| 5 | 0.07 | testPersistenceFailureReportsWarningWithoutUndoingSuccessfulViewChange | kept | 0.07s |

## EVEditingPreferencesTests
`src/mac/AppShellTests/EVEditingPreferencesTests.swift` — 5 tests, 0.5 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.23 | testCaretHoverDefaultsOnPersistsAndTracksSettingsCheckbox | kept | 0.23s |
| 5 | 0.02 | testSmartQuotesDefaultsOffPersistsAndNotifiesOnlyOnChanges | kept | 0.02s |
| 4 | 0.20 | testEditingCategoryCheckboxUpdatesPreferenceWithoutChangingTheme | kept | 0.20s |
| 5 | 0.01 | testTextWidthDefaultsToEightyPersistsRejectsZeroAndNotifiesOnlyOnChanges | kept | 0.01s |
| 5 | 0.09 | testEditingCategoryTextWidthFieldValidatesAndFollowsPreference | kept | 0.09s |

## EVMultilineRenderingTests
`src/mac/Editor/Tests/EVMultilineRenderingTests.swift` — 5 tests, 0.5 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testNativeDocumentWindowLaysOutEveryInitialHardLine | kept | 0.06s |
| 3 | 0.35 | testLayerBackedDocumentWindowCachePaintsEveryInitialHardLine | kept | 0.35s |
| 5 | 0.04 | testLiveRefreshAfterEveryEditKeepsAllPositionedRowsCurrent | kept | 0.04s |
| 5 | 0.04 | testIncrementalEnterProducesTwoPositionedRows | kept | 0.04s |
| 5 | 0.04 | testIncrementalEnterPaintsGlyphPixelsOnBothPositionedRows | kept | 0.04s |

## EVFunctionKeyMappingTests
`src/mac/Editor/Tests/EVFunctionKeyMappingTests.swift` — 7 tests, 0.5 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.07 | testStartupControlF2MappingLeavesExCommandReadyForReturn | kept | 0.07s |
| 5 | 0.10 | testStartupControlF2MappingWithReturnSplitsTheNativeWindow | kept | 0.10s |
| 5 | 0.07 | testFunctionModifiersAndHighFunctionNumbersReachTheirDistinctMappings | kept | 0.07s |
| 5 | 0.07 | testQuotedFunctionKeyKeepsLiteralCaptureAheadOfMappings | kept | 0.07s |
| 5 | 0.08 | testMarkedTextKeepsFunctionKeysInTheInputMethod | kept | 0.08s |
| 5 | 0.07 | testCommandLineTypingMapsAndReplacesTheCurrentSelection | kept | 0.07s |
| 5 | 0.07 | testCommandLineExplicitReplacementAndIMECommitKeepTheirLiteralPayloads | kept | 0.07s |

## EVSettingsKeyboardNavigationTests
`src/mac/AppShellTests/EVSettingsKeyboardNavigationTests.swift` — 4 tests, 0.5 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.23 | testEditingOffersEnabledMarkdownAutodetectionAndPersistsClick | kept | 0.23s |
| 5 | 0.10 | testMarginTabAndBacktabCommitValuesInDisplayedOrder | kept | 0.10s |
| 5 | 0.08 | testInvalidMarginRevertsAndTabStillAdvances | kept | 0.08s |
| 4 | 0.13 | testCategoryChangeCommitsPendingEditAndRebuildsNativeKeyLoop | kept | 0.13s |

## EVSharedDocumentCloseTests
`src/mac/Editor/Tests/EVSharedDocumentCloseTests.swift` — 2 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.22 | testExQuitOfDirtySharedPaneSkipsReviewAndKeepsTheFinalViewProtected | kept | 0.22s |
| 4 | 0.23 | testExQuitOfDirtySharedWindowKeepsTheOtherWindowAndItsEdits | kept | 0.23s |

## EVThemeCatalogueTests
`src/mac/AppShellTests/EVThemeCatalogueTests.swift` — 16 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testNewProfileSeedsFallbackThemesWithoutResourcesAndSelectsMidnight | kept | 0.01s |
| 5 | 0.09 | testBundledThemesRestoreOnlyMissingFilesAndRevertFromCurrentPackage | kept | 0.09s |
| 5 | 0.03 | testDefaultChangesStayInMemoryAndNewThemeCopiesThem | kept | 0.03s |
| 5 | 0.04 | testCatalogueReadsFilesystemAndSortsCaseInsensitively | kept | 0.04s |
| 5 | 0.01 | testInvalidAndDuplicateNamesCannotOverwriteThemes | kept | 0.01s |
| 5 | 0.02 | testMissingAndMalformedSelectedThemesUseDefaultWithoutChangingFiles | kept | 0.02s |
| 5 | 0.02 | testDeletingSelectedThemeFallsBackAndDoesNotReseedIt | kept | 0.02s |
| 5 | 0.02 | testBundledIdentityUsesTheFilenameNotItsDisplayName | kept | 0.02s |
| 5 | 0.01 | testLegacyAppearanceAndStyleFilesAreIgnoredWithoutCreatingATheme | kept | 0.01s |
| 5 | 0.03 | testMalformedLegacySettingsDoNotBlockCurrentThemeOrPreferenceUpdates | kept | 0.03s |
| 5 | 0.03 | testIndependentThemeWritesPreserveOtherSectionsAndFailedWriteDoesNotPublish | kept | 0.03s |
| 5 | 0.02 | testStylePropertyMapsReplaceWholeValuesWhileUnknownFieldsSurvive | kept | 0.02s |
| 5 | 0.02 | testOmittedStyleFamiliesUseBuiltinsAndDotPrefixedThemesRemainSelectable | kept | 0.02s |
| 5 | 0.05 | testSameDisplayNamesRetainTheSelectedFilenameAcrossLaunches | kept | 0.05s |
| 5 | 0.02 | testPreferenceNotificationsDoNotRepublishThemesOrDiscardDefaultEdits | kept | 0.02s |
| 5 | 0.01 | testMalformedThemeDoesNotBlockUnrelatedEditingPreferences | kept | 0.01s |

## EVThemeTests
`src/mac/AppShellTests/EVThemeTests.swift` — 6 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.03 | testThemePersistsSeparatelyAndRejectsInvalidSettings | kept | 0.03s |
| 4 | 0.24 | testMalformedSavedThemeFallsBackAndSettingsWindowFits | kept | 0.24s |
| 5 | 0.04 | testStatusChromeResolvesSparseInheritanceAndIgnoresRemovedAppearanceKeys | kept | 0.04s |
| 5 | 0.05 | testStatusStyleEditsFromMarkdownPublishAllFamiliesAtomically | kept | 0.05s |
| 5 | 0.04 | testCodeDeltaClearingRestoresSharedStatusStyleDefaults | kept | 0.04s |
| 5 | 0.04 | testUnrelatedCodeDeltaEditPreservesSharedStatusStylesWhenPriorDefinitionsAreOmitted | kept | 0.04s |

## EVThemeSelectionTests
`src/mac/AppShellTests/EVThemeSelectionTests.swift` — 8 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.04 | testStyleThemeMenuUsesCatalogueOrderAndSelectsAndCreatesThemes | kept | 0.04s |
| 5 | 0.05 | testThemeSettingsRevertsBundledThemesAndDeletesUserThemes | kept | 0.05s |
| 5 | 0.05 | testFileNamedDefaultRemainsDistinctFromBuiltInDefault | kept | 0.05s |
| 5 | 0.06 | testIdenticallyDisplayedFilenamesRemainIndividuallySelectable | kept | 0.06s |
| 5 | 0.05 | testAppearanceStoresTrackSelectionAndLiveEditsForExistingAndFutureConsumers | kept | 0.05s |
| 5 | 0.06 | testReopeningSettingsRefreshesExternalCatalogueChanges | kept | 0.06s |
| 5 | 0.10 | testOnlyOpeningThemeSettingsRestoresMissingBundledThemes | kept | 0.10s |
| 5 | 0.02 | testOpeningThemeMenuAfterExternalRemovalFallsBackToDefault | kept | 0.02s |

## EVVimOpeningTests
`src/mac/Editor/Tests/EVVimOpeningTests.swift` — 4 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testVimrcUsesTheBundledResourceResolver | kept | 0.06s |
| 4 | 0.13 | testBundledVimRuntimePaintsVimrcByDefault | kept | 0.13s |
| 4 | 0.11 | testBundledMakefileSyntaxOpensAndPaintsWithoutChangingSource | kept | 0.11s |
| 4 | 0.12 | testDocumentFullPathAndSaveAsUpdateVimSetupWithoutEditingSource | kept | 0.12s |

## EVSelectAllTests
`src/mac/Editor/Tests/EVSelectAllTests.swift` — 2 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.22 | testSelectAllCopyCutAndDeleteIncludeTheLastWrappedParagraphForEveryFormatAndLinePolicy | kept | 0.22s |
| 4 | 0.18 | testSelectAllDeleteClearsBlockquoteAndListContextBeforeTyping | kept | 0.18s |

## EVVisibleWhitespaceStyleTests
`src/mac/Editor/Tests/EVVisibleWhitespaceStyleTests.swift` — 5 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.02 | testAllCharacterPropertiesPersistAndClearAtomically | kept | 0.02s |
| 5 | 0.01 | testLiveGestureIsOneUndoAndRedoPreservesUnrelatedSettings | kept | 0.01s |
| 5 | 0.01 | testInvalidMutationsAndWriteFailureLeaveStyleAndHistoryUnchanged | kept | 0.01s |
| 5 | 0.01 | testExternalStyleCommitRefreshesControlsAndDiscardsStaleUndo | kept | 0.01s |
| 3 | 0.33 | testModelessEditorSharesCompactControlsAndHasIndependentUndo | kept | 0.33s |

## EVCaretHoverTests
`src/mac/Editor/Tests/EVCaretHoverTests.swift` — 3 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.23 | testPreviewMatchesPlainClickAcrossModesUnicodeAndBlankSpaceWithoutChangingState | kept | 0.23s |
| 5 | 0.05 | testTypingHidesUntilMovementExceedsThresholdAndPreferenceUpdatesLive | kept | 0.05s |
| 5 | 0.09 | testHoverUsesExistingLargeDocumentGeometryAndRejectsStaleSnapshots | kept | 0.09s |

## EVDocumentNavigationTests
`src/mac/Editor/Tests/EVDocumentNavigationTests.swift` — 6 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.05 | testControlHomeAndEndNavigateDocumentWhileLineSelectorsStayOnLine | kept | 0.05s |
| 5 | 0.04 | testDocumentSelectorsPreserveInsertModeAndNavigatePromptWhenItIsActive | kept | 0.04s |
| 5 | 0.06 | testHomeAndEndUseAppKitKeyBindingsAndControlKeysStayExplicit | kept | 0.06s |
| 5 | 0.05 | testHomeAndEndStillResolveNativeBindingsWithoutCommandModeInputContext | kept | 0.05s |
| 4 | 0.10 | testMarkdownInsertLineSelectorsFollowLinePolicyAndParagraphSelectorsKeepSemanticBoundaries | kept | 0.10s |
| 5 | 0.05 | testShiftVExportsOnlyTheDisplayedRowInVisualLineMode | kept | 0.05s |

## EVCommandPromptEditingTests
`src/mac/Editor/Tests/EVCommandPromptEditingTests.swift` — 7 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.05 | testShiftSelectionClipboardAndNativeReplacementOnlyEditPrompt | kept | 0.05s |
| 5 | 0.05 | testMousePromptSelectionAndContextMenuKeepDocumentSelectionUntouched | kept | 0.05s |
| 5 | 0.06 | testCommandOutputUsesStatusLineWithoutMovingDocumentAndDismissesOnInput | kept | 0.06s |
| 5 | 0.05 | testOutputSelectionCopiesExactMultilineUnicodeAndSurvivesPresentationRefresh | kept | 0.05s |
| 5 | 0.05 | testOutputTimeoutUsesNewMessageDeadlineAndPreservesOtherFocus | kept | 0.05s |
| 5 | 0.05 | testNativeOptionWarningsAndExErrorsReachStatusOutput | kept | 0.05s |
| 5 | 0.05 | testStalePromptSnapshotCannotOverwriteSelection | kept | 0.05s |

## EVFormattedSnapshotIntegrationTests
`src/mac/Editor/Tests/EVFormattedSnapshotIntegrationTests.swift` — 3 tests, 0.4 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.04 | testTextInputAndAccessibilityUseAggregateCountsCachedSlicesAndOnDemandRanges | kept | 0.04s |
| 5 | 0.02 | testBoundedSnapshotRangeMappingAndPointInfoStayIdentityBound | kept | 0.02s |
| 4 | 0.29 | testMillionShortLineRefreshResizeScrollAndEditReadOnlyBoundedProjectionWindows | kept | 0.29s |

## EVIndentationPreferencesTests
`src/mac/AppShellTests/EVIndentationPreferencesTests.swift` — 7 tests, 0.3 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testDefaultsAndIndependentWhitespaceSettingsRoundTrip | kept | 0.01s |
| 4 | 0.16 | testPartialDefaultsRejectNullAndInvalidConfigurationWithoutOverwriting | kept | 0.16s |
| 5 | 0.03 | testWrappedLineIndentUpdatesLiveAndRejectsInvalidValuesAtomically | kept | 0.03s |
| 5 | 0.01 | testKnownStylePropertiesCanBeClearedWhileUnknownNestedKeysSurvive | kept | 0.01s |
| 5 | 0.01 | testSettingsOwnersStaySynchronizedWithExactlyOneNotificationAndAtomicRejection | kept | 0.01s |
| 5 | 0.09 | testEditingControlsValidateAndExposeAllTwelveEntriesAndStyleAction | kept | 0.09s |
| 5 | 0.00 | testListcharsEscapesAndLastEntryWins | kept | 0.00s |

## EVCommandPromptFilenameCompletionTests
`src/mac/Editor/Tests/EVCommandPromptFilenameCompletionTests.swift` — 5 tests, 0.3 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.05 | testNativeTabAndBacktabCycleCaseInsensitiveFilesAndDirectories | kept | 0.05s |
| 5 | 0.06 | testNativeSlashAcceptsDirectoryWithoutDuplicationAndCompletesItsChild | kept | 0.06s |
| 5 | 0.05 | testNativeTypingAndCaretMovementAcceptCompletionAndControlECancelsOnlyActiveCycle | kept | 0.05s |
| 5 | 0.05 | testNativeTextAcceptsCompletedFilenameAndNoMatchLeavesPromptUnchanged | kept | 0.05s |
| 4 | 0.10 | testNativeShiftTabInInsertModeUsesConfiguredTabInsertion | kept | 0.10s |

## EVDocumentReplacementTests
`src/mac/Editor/Tests/EVDocumentReplacementTests.swift` — 4 tests, 0.3 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.07 | testFailedPreparationPreservesDirtySourceHistoryAndEveryLiveView | kept | 0.07s |
| 5 | 0.08 | testOptionalDetectionFailureStillTransfersCoreAndReattachesEveryLiveView | kept | 0.08s |
| 5 | 0.08 | testFailedSecondViewPreparationKeepsLiveViewsAndReleasesTheStagedCore | kept | 0.08s |
| 5 | 0.08 | testReplacementPreservesEachSourceViewsLineMode | kept | 0.08s |

## EVCodePreferencesTests
`src/mac/AppShellTests/EVCodePreferencesTests.swift` — 6 tests, 0.3 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testBundledResourcesFollowRelocationWithoutPersistingPaths | kept | 0.01s |
| 5 | 0.02 | testRetiredSyntaxDirectoryValuesAreIgnoredAndRemovedOnSettingsWrite | kept | 0.02s |
| 5 | 0.02 | testFilenameAssociationsPersistInOrderAndRejectInvalidUpdatesAtomically | kept | 0.02s |
| 5 | 0.03 | testCodeStyleAuthorityDoesNotResurrectDeletedDeclarations | kept | 0.03s |
| 5 | 0.08 | testCanonicalCodeStylesLoadWithoutResurrectingDeclarations | kept | 0.08s |
| 4 | 0.13 | testUnsupportedCodeStyleVersionsStayOnDiskUntilExplicitRestore | kept | 0.13s |

## EVDocumentLifecycleTests
`src/mac/AppShellTests/EVDocumentLifecycleTests.swift` — 29 tests, 0.3 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testNativeSavePanelHasCurrentWritableType | kept | 0.00s |
| 5 | 0.03 | testOrdinarySaveKeepsCodeDotfileAndExtensionlessNames | kept | 0.03s |
| 5 | 0.01 | testOrdinarySaveToAnotherURLPreservesOriginalAndAdoptsNewDestination | kept | 0.01s |
| 5 | 0.01 | testDirectSafeWriteToAnotherURLPreservesOriginalAndDoesNotAdoptCopy | kept | 0.01s |
| 5 | 0.01 | testOrdinarySaveToAnotherHardLinkPreservesOriginalBytesAndName | kept | 0.01s |
| 5 | 0.00 | testSaveRejectsDifferentSymlinkSpellingsAndPreservesOriginalBytes | kept | 0.00s |
| 5 | 0.01 | testOrdinarySaveToUnchangedSymlinkNameRemainsSupported | kept | 0.01s |
| 5 | 0.00 | testSaveRejectsCaseOnlyAliasOnCaseInsensitiveVolume | kept | 0.00s |
| 5 | 0.01 | testNativeMoveWritesCopyAndFailedMovePreservesBothOriginalAndBinding | kept | 0.01s |
| 5 | 0.00 | testSameSerializationPresentationChangesKeepOrdinarySave | kept | 0.00s |
| 5 | 0.00 | testNativeDocumentRegistrationAcceptsUnknownFilesAsReadableData | kept | 0.00s |
| 5 | 0.00 | testUnknownReadTypesUseTextWithoutBecomingWritableFormats | kept | 0.00s |
| 5 | 0.00 | testNSDocumentDataOverridesDelegateReadAndSameFormatSerializationToSourceBackend | kept | 0.00s |
| 5 | 0.00 | testNSDocumentDataRejectsUnsafeTypesBeforeBackendSerialization | kept | 0.00s |
| 5 | 0.00 | testNativeURLReadAndWriteFlowThroughNSDocumentDataBoundary | kept | 0.00s |
| 5 | 0.01 | testFailedURLReadDoesNotRecordARecentDocument | kept | 0.01s |
| 5 | 0.00 | testBackendChangeNotificationMarksNativeDocumentEdited | kept | 0.00s |
| 5 | 0.00 | testCorePersistenceStateIsTheFinalNativeEditedAuthority | kept | 0.00s |
| 5 | 0.00 | testPrintStaysDisabledUntilThereIsAPaginatedProjection | kept | 0.00s |
| 5 | 0.00 | testOnlyAPristineUntitledLaunchPlaceholderMayBeDiscardedForAnOpenedFile | kept | 0.00s |
| 4 | 0.12 | testOpenRecentReplacesThePristineLaunchPlaceholder | kept | 0.12s |
| 5 | 0.01 | testNativeSaveAcknowledgesExactSnapshotOnlyAfterSuccessfulWrite | kept | 0.01s |
| 5 | 0.00 | testNativeSaveAsRejectsCrossFormatBeforeSnapshotOrFileMutation | kept | 0.00s |
| 5 | 0.00 | testFailedNativeWriteDoesNotAcknowledgeOrClearDirtyState | kept | 0.00s |
| 5 | 0.01 | testSuccessfulFileWriteRemainsRecentWhenCoreAcknowledgementIsStale | kept | 0.01s |
| 5 | 0.00 | testSaveToWritesCopyWithoutMovingTheCoreSavePoint | kept | 0.00s |
| 5 | 0.00 | testSnapshotPreparationFailureDoesNotStartNativeWriteOrAcknowledge | kept | 0.00s |
| 5 | 0.00 | testRecoveryAutosavesDoNotRecordRecentDocumentsOrSaveSource | kept | 0.00s |
| 5 | 0.03 | testMakeWindowControllersCreatesOneSurfacePerNativeDocumentWindow | kept | 0.03s |

## EVPresentationExportIntegrationTests
`src/mac/Editor/Tests/EVPresentationExportIntegrationTests.swift` — 5 tests, 0.3 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testModeCaretPathsAreExclusiveAndInactiveEditorUsesOutline | kept | 0.06s |
| 5 | 0.06 | testCommandLineExportAndNativeRenderStateTrackUTF8Cursor | kept | 0.06s |
| 5 | 0.05 | testCharacterAndLineSelectionsUseExactLogicalAndLayoutExports | kept | 0.05s |
| 5 | 0.05 | testBlockSelectionKeepsDiscontiguousSegmentsAndExactRectangles | kept | 0.05s |
| 5 | 0.06 | testBidiSelectionPaintsEveryExactExportedRectangle | kept | 0.06s |

## EVFileFingerprintTests
`src/mac/AppShellTests/EVFileFingerprintTests.swift` — 3 tests, 0.3 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.25 | testStreamingDigestMatchesBytesAcrossMultipleChunksAndEmptyFiles | kept | 0.25s |
| 5 | 0.01 | testOpenedDescriptorKeepsIdentityWhenPathIsReplaced | kept | 0.01s |
| 5 | 0.00 | testDeletedMissingAndUnreadableDestinationsStayDistinct | kept | 0.00s |

## EVLiteralInputTests
`src/mac/Editor/Tests/EVLiteralInputTests.swift` — 3 tests, 0.3 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.12 | testBothPrefixesQuotePhysicalTabReturnEscapeAndControlCharacters | kept | 0.12s |
| 5 | 0.05 | testNativeControlValueIsPreservedWhenThePrintedKeyDiffers | kept | 0.05s |
| 5 | 0.09 | testNormalModeRetainsVisualBlockShortcuts | kept | 0.09s |

## EVStatusFilePathTests
`src/mac/AppShellTests/EVStatusFilePathTests.swift` — 6 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testPathsBelowACommonDirectoryStayRelative | kept | 0.00s |
| 5 | 0.00 | testCopiedRelativePathTraversesRootAndKeepsTheWholeFilename | kept | 0.00s |
| 5 | 0.00 | testFittingDropsDirectoriesBeforeClippingFilenameAtGraphemeBoundaries | kept | 0.00s |
| 4 | 0.21 | testFilenameMenuCopiesCurrentBindingAndWorkingDirectoryWithoutUsingDisplayText | kept | 0.21s |
| 5 | 0.01 | testPathStartsAtTheSamePositionInEveryModeWhenThereIsRoom | kept | 0.01s |
| 5 | 0.02 | testNarrowingHidesModeThenDirectoriesThenClipsFilenameWhilePositionRemains | kept | 0.02s |

## EVLinksTests
`src/mac/Editor/Tests/EVLinksTests.swift` — 6 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testURLsAreStructuredAndNeverReinterpretedAsCommands | kept | 0.00s |
| 5 | 0.00 | testLinkRoutesSeparateHeadingsDocumentsAndWebDestinations | kept | 0.00s |
| 5 | 0.05 | testHeadingLinkNavigatesCurrentDocumentWithoutChangingSourceAndRejectsStaleTarget | kept | 0.05s |
| 5 | 0.07 | testLineLinkUsesPhysicalSourceLineAndCenters | kept | 0.07s |
| 5 | 0.05 | testContextMenuOpensCurrentLinkAndRejectsStaleMenuWithoutMovingSelection | kept | 0.05s |
| 5 | 0.05 | testPointerMenuOnlyAddsLinkForPaintedAnchorContent | kept | 0.05s |

## EVArabicTypographyTests
`src/mac/Editor/Tests/EVArabicTypographyTests.swift` — 2 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.09 | testLongArabicAndMixedBidiWrapWithinTheViewportAtLegalBoundaries | kept | 0.09s |
| 4 | 0.12 | testLargeArabicDocumentRetainsUnchangedRenderResourcesAndInvalidatesFontGeneration | kept | 0.12s |

## EVConfigurationStoreTests
`src/mac/AppShellTests/EVConfigurationStoreTests.swift` — 12 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testMarkdownAutodetectDefaultsOnPersistsAndUpdatesLivePreferences | kept | 0.01s |
| 5 | 0.00 | testInvalidMarkdownAutodetectDoesNotOverwriteSettings | kept | 0.00s |
| 5 | 0.03 | testLegacyPreferencesMigrateWhileTheLegacyThemeIsIgnored | kept | 0.03s |
| 5 | 0.01 | testUnknownKeysSurviveUpdatesAndIndependentStoresMerge | kept | 0.01s |
| 5 | 0.02 | testMarkdownFormattedViewDefaultsOffAndPersistsWithoutLosingOtherPreferences | kept | 0.02s |
| 5 | 0.02 | testInvalidMarkdownFormattedViewValuesAreRejectedWithoutOverwriting | kept | 0.02s |
| 5 | 0.01 | testInvalidVersionValuesAndMalformedJSONAreNeverOverwritten | kept | 0.01s |
| 5 | 0.01 | testWriteFailureDoesNotPublishSettingsChange | kept | 0.01s |
| 5 | 0.01 | testInvalidExternalConfigurationChangeIsReportedWithoutPublishing | kept | 0.01s |
| 5 | 0.01 | testTextWidthDefaultsToEightyPersistsAndPreservesUnrelatedSettings | kept | 0.01s |
| 5 | 0.03 | testInvalidTextWidthValuesAreRejectedWithoutOverwriting | kept | 0.03s |
| 5 | 0.03 | testThemeStyleSheetsAreFormatSpecificVersionedAndPreserveUnknownData | kept | 0.03s |

## EVSubstituteConfirmationTests
`src/mac/Editor/Tests/EVSubstituteConfirmationTests.swift` — 3 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testNativeChoicesDisplayPromptAndCurrentMatchThenPublishOneUndoUnit | kept | 0.06s |
| 5 | 0.06 | testNativeEscapeCommitsPriorApprovalsAndNoApprovalCancelsCleanly | kept | 0.06s |
| 5 | 0.08 | testOtherPaneEditRejectsStaleApprovalsWithoutChangingItsText | kept | 0.08s |

## EVRecentFilesTests
`src/mac/AppShellTests/EVRecentFilesTests.swift` — 6 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.05 | testMostRecentTenFullPathsPersistAndOpeningAnExistingEntryPromotesIt | kept | 0.05s |
| 5 | 0.02 | testSymlinksAndHardLinksReferToOneRecentFileButMatchingNamesDoNot | kept | 0.02s |
| 5 | 0.02 | testIndependentStoresMergeFreshOrderClearPersistentlyAndKeepUnknownSettings | kept | 0.02s |
| 5 | 0.02 | testRecordingTheFirstFileRefreshesStaleMemoryWithoutRewritingConfig | kept | 0.02s |
| 5 | 0.01 | testFailedWritesKeepPublishedRecentFilesAndReportTheFailure | kept | 0.01s |
| 5 | 0.07 | testMalformedRecentFileSettingsAndInvalidPathsAreNeverWrittenOver | kept | 0.07s |

## EVCommandLineRegisterTests
`src/mac/Editor/Tests/EVCommandLineRegisterTests.swift` — 2 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.13 | testNativeRegisterSelectorUsesCoreInsteadOfReplacingPromptText | kept | 0.13s |
| 5 | 0.05 | testLiteralRegisterVariantAndCancellationKeepPromptEditingActive | kept | 0.05s |

## EVDocumentModeMenuTests
`src/mac/Editor/Tests/EVDocumentModeMenuTests.swift` — 3 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.08 | testMenusShowDetectedLanguageAlignChecksAndApplyOverridesWithoutChangingSource | kept | 0.08s |
| 5 | 0.09 | testAutoFallbackAndMarkdownCodeDistinctionAndTrackedItemIdentity | kept | 0.09s |
| 5 | 0.01 | testFlyoutRetainsCapturedRevisionWhileParentTracksAndDisablesWithoutDocument | kept | 0.01s |

## EVFileDropTests
`src/mac/Editor/Tests/EVFileDropTests.swift` — 5 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testFileURLsKeepOrderAndUnicodeNamesWithoutTreatingStringsAsPaths | kept | 0.00s |
| 5 | 0.04 | testFinderDropRoutesToItsExactSurfaceWithoutInsertingPathsOrEditingSource | kept | 0.04s |
| 5 | 0.07 | testDropsRequireAnAttachedHostAndIgnoreFoldersAndText | kept | 0.07s |
| 5 | 0.04 | testOpenFailureIsShownWithoutChangingTheOriginalDocument | kept | 0.04s |
| 5 | 0.04 | testMarkedInputIsCommittedBeforeTheHostChecksWhetherReplacementIsSafe | kept | 0.04s |

## EVScrollWheelTests
`src/mac/Editor/Tests/EVScrollWheelTests.swift` — 3 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testVerticalWheelUsesAppKitDirectionAndUnitsAndClampsAtBothEnds | kept | 0.06s |
| 5 | 0.08 | testHorizontalWheelUsesAppKitDirectionAndUnitsClampsAndHonorsWrap | kept | 0.08s |
| 5 | 0.04 | testWrappedUnbreakableWordKeepsHorizontalScrollingAvailable | kept | 0.04s |

## EVRecoveryCoreTests
`src/mac/Editor/Tests/EVRecoveryCoreTests.swift` — 4 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testRecoveryRestoresPhysicalBytesAndForcedInterpretationAndRemainsDirty | kept | 0.01s |
| 5 | 0.10 | testReloadPreservesEachViewPositionAndClampsAfterTruncation | kept | 0.10s |
| 5 | 0.02 | testReadOnlyIsBufferPolicyWithoutChangingSourceOrRevision | kept | 0.02s |
| 5 | 0.04 | testReadOnlyWritePublishesE45AndReturnsToNormalMode | kept | 0.04s |

## EVFormattingToolbarPerformanceTests
`src/mac/Editor/Tests/EVFormattingToolbarPerformanceTests.swift` — 1 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 4 | 0.15 | testLargeDocumentToolbarRefreshAndFormattingToggles | kept | 0.15s |

## EVCaretCellTests
`src/mac/Editor/Tests/EVCaretCellTests.swift` — 3 tests, 0.2 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testEndOfLineDrawsTheLastCharacterCellNotTheOneBeforeIt | kept | 0.06s |
| 5 | 0.05 | testEveryNormalModePositionDrawsTheClusterUnderTheCursor | kept | 0.05s |
| 5 | 0.04 | testInsertModeDrawsAnInsertionBoundary | kept | 0.04s |

## EVZoomIntegrationTests
`src/mac/Editor/Tests/EVZoomIntegrationTests.swift` — 2 tests, 0.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.07 | testMenuStopsSaturateAtTwentyFiveAndFiveHundredPercentWithoutEditing | kept | 0.07s |
| 5 | 0.08 | testCommandZoomKeysAndMenuResponderWorkInNormalAndInsertWithoutText | kept | 0.08s |

## EVMenuBuilderTests
`src/mac/AppShellTests/EVMenuBuilderTests.swift` — 23 tests, 0.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testTopLevelMenuOrderMatchesSpecification | kept | 0.01s |
| 5 | 0.01 | testAllAndOnlySpecifiedKeyEquivalentsAreInstalled | kept | 0.01s |
| 5 | 0.01 | testEveryMenuCommandAppearsExactlyOnceAndUsesItsExpectedRoute | kept | 0.01s |
| 5 | 0.00 | testListShortcutMatcherUsesTheActiveKeyboardLayoutForShiftedDigits | kept | 0.00s |
| 5 | 0.02 | testCommandAUsesNativeNumericFieldEditorInsteadOfDocumentCommands | kept | 0.02s |
| 5 | 0.01 | testNativeDocumentItemsUseNSDocumentSelectorsAndSemanticTags | kept | 0.01s |
| 5 | 0.01 | testApplicationAndWindowLifecycleActionsUseExpectedTargets | kept | 0.01s |
| 5 | 0.01 | testShellAndSystemOwnedItemsUseExpectedSelectors | kept | 0.01s |
| 5 | 0.01 | testRecentDocumentsAreRebuiltWithURLsAndClearBehavior | kept | 0.01s |
| 5 | 0.01 | testEmptyRecentDocumentsMenuContainsOnlyDisabledClearCommand | kept | 0.01s |
| 5 | 0.01 | testRecentDocumentsUseMinimalUniqueSuffixesForEveryDuplicateName | kept | 0.01s |
| 5 | 0.01 | testRecentDocumentsDisambiguateUnequalDepthsWithoutDuplicateRootSeparators | kept | 0.01s |
| 5 | 0.01 | testRecentDocumentsPreserveUnicodeSpacesAndExactFilenameExtensions | kept | 0.01s |
| 5 | 0.01 | testEveryCoreMenuItemHasAStableTypedCommandTag | kept | 0.01s |
| 5 | 0.01 | testNoBareViemTextKeyIsRegisteredAsAGlobalShortcut | kept | 0.01s |
| 5 | 0.01 | testTextEncodingSubmenuHasTheFourFormerStatusChoicesAndStableActions | kept | 0.01s |
| 5 | 0.01 | testLineNumberCheckboxPrecedesInvisibleCharactersInViewMenu | kept | 0.01s |
| 5 | 0.01 | testStyleMenusReserveOneNativeCheckmarkColumnForEveryRow | kept | 0.01s |
| 5 | 0.01 | testStyleMenusRefreshFromCurrentCatalogueWithBaseFirstAndStableIDs | kept | 0.01s |
| 5 | 0.01 | testCodeStyleMenuPreservesEditAndExplicitDefinitionActions | kept | 0.01s |
| 5 | 0.01 | testStyleMenuRequeriesProviderRatherThanCachingDefinitionsOrIndexes | kept | 0.01s |
| 5 | 0.01 | testStyleMenuDoesNotReplaceTrackedItems | kept | 0.01s |
| 5 | 0.01 | testUnavailableStyleProviderLeavesHonestDisabledFallback | kept | 0.01s |

## EVEditingAssistanceIntegrationTests
`src/mac/Editor/Tests/EVEditingAssistanceIntegrationTests.swift` — 2 tests, 0.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.06 | testNativeMarkdownTypingTracksLivePreferenceAndLiteralNext | kept | 0.06s |
| 5 | 0.06 | testLiveSmartQuotePreferenceOnlyAffectsSubsequentTypedInput | kept | 0.06s |

## EVViewPreferencesTests
`src/mac/AppShellTests/EVViewPreferencesTests.swift` — 2 tests, 0.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.02 | testMarginsPersistIndependentlyOfThemeAndRejectInvalidValues | kept | 0.02s |
| 5 | 0.09 | testViewCategoryReplacesDocumentsAndEditsEveryMargin | kept | 0.09s |

## EVRecoveryStoreTests
`src/mac/AppShellTests/EVRecoveryStoreTests.swift` — 15 tests, 0.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testStaleRecoveryRetiresOnlyAfterNewSnapshotCommits | kept | 0.01s |
| 5 | 0.01 | testLiveForeignRemoteInvalidAndSymlinkCandidatesCannotBeDeleted | kept | 0.01s |
| 5 | 0.01 | testIdenticalBytesInReplacementInodeAreNotRetired | kept | 0.01s |
| 5 | 0.01 | testChangedBytesInSameInodeAreNotRetiredAndFailurePersists | kept | 0.01s |
| 5 | 0.01 | testFailedPublicationRetainsCandidateUntilLaterSuccessfulWrite | kept | 0.01s |
| 5 | 0.01 | testSupersededWriteCannotRetireCandidate | kept | 0.01s |
| 5 | 0.01 | testContendedCleanupIsNonblockingAndDoesNotRetryAfterUnlock | kept | 0.01s |
| 5 | 0.01 | testConcurrentCleanersPreserveBothNewSessions | kept | 0.01s |
| 5 | 0.01 | testExclusiveSlotsPreserveExistingViemAndForeignVimFiles | kept | 0.01s |
| 5 | 0.01 | testHardLinkedSnapshotRemainsRecoverableButCannotAuthorizeDeletion | kept | 0.01s |
| 5 | 0.00 | testZeroByteSlotAndCollidingStagingFileArePreserved | kept | 0.00s |
| 5 | 0.01 | testSnapshotRoundTripsPhysicalBytesAndInterpretationMetadata | kept | 0.01s |
| 5 | 0.01 | testQueuedOlderSnapshotsCannotReplaceLatestAndClosedStoreCannotRecreateSlot | kept | 0.01s |
| 5 | 0.00 | testReplacementByAnotherOwnerIsNeverOverwrittenOrDeleted | kept | 0.00s |
| 5 | 0.01 | testSymlinkUsesTargetRecoveryIdentityAndUnknownVersionIsNotRecoverable | kept | 0.01s |

## EVDocumentWindowPlacementTests
`src/mac/AppShellTests/EVDocumentWindowPlacementTests.swift` — 10 tests, 0.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.02 | testSavedFrameRestoresExactlyAcrossPlacementInstances | kept | 0.02s |
| 5 | 0.00 | testGutterGrowthAndRemovalPreserveContentWidthAndScreenOriginWhenRoomExists | kept | 0.00s |
| 5 | 0.04 | testRepeatedRestorationWithDifferentGuttersDoesNotGrowSavedContent | kept | 0.04s |
| 5 | 0.00 | testGutterUsesRightSpaceOnlyWhenLeftSpaceRunsOutAndClampsToMonitor | kept | 0.00s |
| 5 | 0.00 | testOffscreenFrameMovesBeforeAnyDimensionShrinks | kept | 0.00s |
| 5 | 0.00 | testOnlyOversizedDimensionsShrinkAndTheWholeFrameFits | kept | 0.00s |
| 5 | 0.00 | testRestorationSelectsExistingDisplayOrNearestAfterDisconnection | kept | 0.00s |
| 5 | 0.02 | testEverySubsequentWindowUsesNativeCascadePointAndLatestRecordedFrame | kept | 0.02s |
| 5 | 0.01 | testDefaultSizeAndEmptyDesktopRemainUsableBeforeScreenAssignment | kept | 0.01s |
| 5 | 0.02 | testConfigurationPreservesOtherPreferencesAndRejectsInvalidGeometry | kept | 0.02s |

## EVExListOutputTests
`src/mac/Editor/Tests/EVExListOutputTests.swift` — 1 tests, 0.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.09 | testListEscapesLiteralControlsWhileNumberKeepsRawLogicalContent | kept | 0.09s |

## EVCodeStyleLoadingTests
`src/mac/Editor/Tests/EVCodeStyleLoadingTests.swift` — 2 tests, 0.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.04 | testMalformedSelectedThemeFallsBackWithoutRewritingTheFile | kept | 0.04s |
| 5 | 0.03 | testRepairedThemeCanBeSelectedWithoutRewritingExternalContents | kept | 0.03s |

## EVInactiveCaretTests
`src/mac/Editor/Tests/EVInactiveCaretTests.swift` — 2 tests, 0.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.05 | testWindowApplicationAndPaneFocusStopAndRestoreEveryCaretMode | kept | 0.05s |
| 5 | 0.00 | testInactiveOutlineRetainsModeGeometry | kept | 0.00s |

## EVWhitespaceViewportTests
`src/mac/Editor/Tests/EVWhitespaceViewportTests.swift` — 2 tests, 0.1 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.05 | testCachedHorizontalScrollRejectsCountedMarkerBatchFromOldViewport | kept | 0.05s |
| 5 | 0.00 | testMarkerViewportRegionsAreValidatedBeforeOutputWrites | kept | 0.00s |

## EVFormattingToolbarChromeTests
`src/mac/AppShellTests/EVFormattingToolbarChromeTests.swift` — 2 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.02 | testVisibilityFollowsFormatPersistsAndDoesNotLeakAcrossPanes | kept | 0.02s |
| 5 | 0.02 | testSettingsMergeAndRejectNonBooleanVisibility | kept | 0.02s |

## EVCompletionPopupLayoutTests
`src/mac/Editor/Tests/EVCompletionPopupLayoutTests.swift` — 4 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.02 | testNativeTextLeadingEdgeMatchesWordAnchorAndDoesNotFollowSelection | kept | 0.02s |
| 5 | 0.02 | testRightToLeftPopupMirrorsTextEdgeWritingDirectionAndScrollbar | kept | 0.02s |
| 5 | 0.00 | testGrowingResultsKeepTheSameSideOfAnUnchangedWordAnchor | kept | 0.00s |
| 5 | 0.00 | testFrameClampsMirroredTextAlignmentOnlyAtScreenBoundary | kept | 0.00s |

## EVStartupFileTests
`src/mac/AppShellTests/EVStartupFileTests.swift` — 9 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testMissingStartupFileIsCreatedEmptyInNewAndExistingProfiles | kept | 0.00s |
| 5 | 0.00 | testUTF8BOMIsRemovedAndAllOtherContentsRemainUnchanged | kept | 0.00s |
| 5 | 0.00 | testInvalidUTF8ReportsTheFilePathWithoutReturningPartialCommands | kept | 0.00s |
| 5 | 0.00 | testUnreadableStartupPathReportsTheFilePath | kept | 0.00s |
| 5 | 0.00 | testFIFOStartupPathIsRejectedWithoutWaitingForAWriter | kept | 0.00s |
| 5 | 0.00 | testSymlinkToRegularStartupFileRemainsSupported | kept | 0.00s |
| 5 | 0.00 | testSizeLimitAcceptsOneMiBAndRejectsAnAdditionalByte | kept | 0.00s |
| 5 | 0.02 | testConfigurationSharesOneStartupSnapshotUntilReopened | kept | 0.02s |
| 5 | 0.01 | testMissingStartupSnapshotRemainsEmptyWhenFileIsCreatedLater | kept | 0.01s |

## EVTablePickerTests
`src/mac/Editor/Tests/EVTablePickerTests.swift` — 4 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testHoverNeverGrowsAndDraggingClampsEachDimensionIndependently | kept | 0.00s |
| 5 | 0.00 | testKeyboardCanReachEveryPickerSizeAndRetainsExpandedExtent | kept | 0.00s |
| 5 | 0.02 | testToolbarReleaseOpensWithoutInsertingThenCellReleaseAccepts | kept | 0.02s |
| 5 | 0.02 | testToolbarDragThresholdAndAboveLeftReleaseCancel | kept | 0.02s |

## EVProfileDirectoryTests
`src/mac/AppShellTests/EVProfileDirectoryTests.swift` — 3 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testDefaultProfileAndOverridePrecedence | kept | 0.00s |
| 5 | 0.02 | testSettingsAndStylesUseTheResolvedProfile | kept | 0.02s |
| 5 | 0.01 | testInjectedHomeSeedsItsOwnThemeCatalogue | kept | 0.01s |

## EVCommandQuitApplicationTests
`src/mac/AppShellTests/EVCommandQuitApplicationTests.swift` — 2 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testQuitChecksRemainingDocumentWindowsAndDoesNotLatchOrdinaryClose | kept | 0.00s |
| 5 | 0.02 | testOtherOrdinaryWindowsAndNewlyOpenedDocumentsPreventTermination | kept | 0.02s |

## EVStatusLineVisibilityTests
`src/mac/AppShellTests/EVStatusLineTests.swift` — 1 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testAHiddenStatusLineAppearsForACommandLineAndHidesAgain | kept | 0.01s |

## EVExFileWriterTests
`src/mac/AppShellTests/EVExFileWriterTests.swift` — 1 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.01 | testExclusiveWriteAndLateExternalChangeKeepPriorBytes | kept | 0.01s |

## EVStatusBarStateTests
`src/mac/AppShellTests/EVStatusBarStateTests.swift` — 2 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testNewDocumentStatusDefaults | kept | 0.00s |
| 5 | 0.00 | testStatusBarDisplaysModeFileMessageAndLocation | kept | 0.00s |

## EVKerningTests
`src/mac/Editor/Tests/EVKerningTests.swift` — 1 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testPreviewZeroLetterSpacingAndImportedKerningSwitchKeepNativeKerning | kept | 0.00s |

## EVCommandLineLaunchTests
`src/mac/AppShellTests/EVCommandLineLaunchTests.swift` — 3 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testLaunchEnvelopePreservesPortableArgumentsAndConnectsOnlyAnAcceptanceReceipt | kept | 0.00s |
| 5 | 0.00 | testInvalidArgumentsAndMalformedEnvelopeAreRejectedBeforeAcceptance | kept | 0.00s |
| 5 | 0.00 | testOrdinaryLaunchHasNoAcceptanceOrDocumentCompletion | kept | 0.00s |

## EVCaretAppearanceResolverTests
`src/mac/Editor/Tests/EVCaretAppearanceResolverTests.swift` — 2 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testSystemAndAccessibilityChangesAdvanceGenerationAndPublish | kept | 0.00s |
| 5 | 0.00 | testBlockGlyphColorChoosesTheHigherWCAGContrastForRepresentativeColors | kept | 0.00s |

## EVExternalFileReviewStateTests
`src/mac/AppShellTests/EVExternalFileReviewStateTests.swift` — 3 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testReturnToBaselineClearsAcknowledgementWithoutReleasingPendingReview | kept | 0.00s |
| 5 | 0.00 | testBridgePreservesReviewActionsAcknowledgementAndReset | kept | 0.00s |
| 5 | 0.00 | testBridgeRetriesFailedReviewAndIgnoresOtherCompletion | kept | 0.00s |

## EVFontVariationsTests
`src/mac/CoreTextProvider/Tests/EVFontVariationsTests.swift` — 2 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testModifiersRestoreBaseCoordinatesAndClampToFontLimits | kept | 0.00s |
| 5 | 0.00 | testItalicAxisAndStyleLinkedWeightUseDesignedTargets | kept | 0.00s |

## EVCustomCaretBlinkControllerTests
`src/mac/Editor/Tests/EVCustomCaretBlinkControllerTests.swift` — 5 tests, 0.0 s; deleting 0

| Score | Time (s) | Test | Action | Note |
| --- | --- | --- | --- | --- |
| 5 | 0.00 | testActiveCaretBlinksWithInjectedClockAndStopCancelsIt | kept | 0.00s |
| 5 | 0.00 | testActivityRestartsVisiblePhaseAndInactiveStateIsSteadyOutline | kept | 0.00s |
| 5 | 0.00 | testReduceMotionMakesActiveCaretSteadyAndRestoresBlinkingWhenDisabled | kept | 0.00s |
| 5 | 0.00 | testDifferentiateWithoutColorRefreshesPresentationWithoutChangingCadence | kept | 0.00s |
| 5 | 0.00 | testStoppedControllerIgnoresActivityAndActiveStateUntilStarted | kept | 0.00s |
