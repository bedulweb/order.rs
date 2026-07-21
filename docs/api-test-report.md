# API live test results

Date: 2026-07-21 · Session: valid `muc_token` · Base: `https://www.bigseller.com`

**Score: 52/52 PASS**

| Status | Method | Path | code | ms | data |
|--------|--------|------|------|----|------|
| ✅ | `GET` | `/api/v1/3pl/shipping/list.json` | 0 | 107 | list[6] |
| ✅ | `GET` | `/api/v1/alert/homeAlertInfo.json` | 0 | 137 | keys=['count', 'alertActionVOS', 'systemMessages'] |
| ✅ | `GET` | `/api/v1/announcement/systemAndPlatform.json` | 0 | 70 | keys=['systemList', 'marketplaceList'] |
| ✅ | `GET` | `/api/v1/checkUserOrderDeductionFree.json` | 0 | 80 | keys=['expireTime', 'expireDays', 'showInfo', 'vip |
| ✅ | `GET` | `/api/v1/clickNewState.json` | 0 | 54 | null |
| ✅ | `GET` | `/api/v1/common/toolkit/checkFeatureAccess.json` | 0 | 69 | bool=True |
| ✅ | `GET` | `/api/v1/common/toolkit/stat/offline.json` | 0 | 55 | null |
| ✅ | `GET` | `/api/v1/dashboard/orderInventoryCount.json` | 0 | 100 | keys=['inventory', 'order'] |
| ✅ | `GET` | `/api/v1/distributor/distributor/getNewMessageRemind.json` | 0 | 53 | list[2] |
| ✅ | `GET` | `/api/v1/distributor/getDistributorRole.json` | 0 | 54 | keys=['distributor', 'supplier'] |
| ✅ | `GET` | `/api/v1/distributor/supplier/getNewMessageRemind.json` | 0 | 52 | list[2] |
| ✅ | `GET` | `/api/v1/expiredShops.json` | 0 | 74 | keys=[] |
| ✅ | `GET` | `/api/v1/getFunctionWhite.json` | 0 | 47 | bool=True |
| ✅ | `GET` | `/api/v1/getHelpDocumentContentConfig.json` | 0 | 112 | null |
| ✅ | `GET` | `/api/v1/getQuickOperate.json` | 0 | 53 | null |
| ✅ | `GET` | `/api/v1/getRecommendFunction.json` | 0 | 66 | list[5] |
| ✅ | `GET` | `/api/v1/goods/getPaidGoodsNum.json` | 0 | 69 | keys=['facebook_page_count', 'order_count', 'shop_ |
| ✅ | `GET` | `/api/v1/goods/quotaDetection.json` | 0 | 65 | keys=['overDueType', 'orderLimitDays', 'shopLimitN |
| ✅ | `GET` | `/api/v1/helps.json` | 0 | 73 | keys=['videos', 'helps'] |
| ✅ | `GET` | `/api/v1/index.json` | 0 | 365 | keys=['user', 'userSite', 'masterAccount', 'authSh |
| ✅ | `GET` | `/api/v1/inventorySetting/list.json` | 0 | 53 | keys=['multipleUnitsFlag', 'autoMappingSamePlatfor |
| ✅ | `GET` | `/api/v1/isLogin.json` | 0 | 51 | bool=True |
| ✅ | `GET` | `/api/v1/lang/getLang.json` | 0 | 36 | str=en_US |
| ✅ | `GET` | `/api/v1/load/afterList.json` | 0 | 47 | list[3] |
| ✅ | `GET` | `/api/v1/newMessages.json` | 0 | 138 | list[0] |
| ✅ | `GET` | `/api/v1/order/constant/queryLogisticsServices.json` | 0 | 62 | list[7] |
| ✅ | `GET` | `/api/v1/order/getNewOrderMessageRemind.json` | 0 | 58 | keys=['code', 'orderCount'] |
| ✅ | `GET` | `/api/v1/order/refreshSyncCheck.json` | 0 | 53 | int=0 |
| ✅ | `GET` | `/api/v1/order/searchConfig/getSearchConfigsAndUncheckedIds.json` | 0 | 55 | keys=['allConfigs', 'uncheckedIds'] |
| ✅ | `GET` | `/api/v1/order/tiktok/getFailReason.json` | 0 | 53 | list[9] |
| ✅ | `GET` | `/api/v1/order/v2/filterList.json` | 0 | 68 | bool=True |
| ✅ | `GET` | `/api/v1/orderSettings/template/queryPickTemplateList.json` | 0 | 58 | list[1] |
| ✅ | `GET` | `/api/v1/scrollMessages.json` | 0 | 113 | list[2] |
| ✅ | `GET` | `/api/v1/setting/column/productCustomizeNavigationList.json` | 0 | 56 | keys=['shopee', 'lazada', 'tiktok', 'tokopedia', ' |
| ✅ | `GET` | `/api/v1/setting/config/getUserChooseConfigs.json` | 0 | 50 | keys=[] |
| ✅ | `GET` | `/api/v1/shop/checkShop/auth/invalid.json` | 0 | 51 | keys=['code'] |
| ✅ | `GET` | `/api/v1/shop/group/page.json` | 0 | 53 | list[0] |
| ✅ | `GET` | `/api/v1/shop/health/notification.json` | 0 | 60 | keys=['lazada', 'shopee'] |
| ✅ | `GET` | `/api/v1/shopsAndPlatforms.json` | 0 | 57 | keys=['shopeeGlobalShops', 'filterPlatforms', 'tik |
| ✅ | `GET` | `/api/v3/account/userRights.json` | 0 | 59 | keys=['version', 'subAdmin', 'userScopeMap'] |
| ✅ | `GET` | `/api_v2/api/v2/genVerifyCode.json` | 0 | 45 | keys=['level', 'accessCode', 'reason', 'base64Imag |
| ✅ | `POST` | `/api/v1/image/getAlbumUserSummary.json` | 0 | 77 | keys=['allowSize', 'allowSizeStr', 'usedSize', 'us |
| ✅ | `POST` | `/api/v1/order/enableOrderNumSort.json` | 0 | 59 | bool=True |
| ✅ | `POST` | `/api/v1/order/getOrderStatusCount.json` | 0 | 140 | keys=['redisKey', 'mode', 'warehouseMap', 'shopLis |
| ✅ | `POST` | `/api/v1/order/new/pageList.json` | 0 | 166 | keys=['page', 'state', 'userTag', 'warehouseUsedSt |
| ✅ | `POST` | `/api/v1/order/wave/getWaveSippedUsedState.json` | 0 | 66 | int=0 |
| ✅ | `POST` | `/api/v1/orderSalesStatistics.json` | 0 | 153 | list[1] |
| ✅ | `POST` | `/api/v1/orderSettings/other/index.json` | 0 | 130 | keys=['autoHighPrintState', 'lazadaRtsState', 'aut |
| ✅ | `POST` | `/api/v1/queryCondition/getOrderSearchTag.json` | 0 | 62 | null |
| ✅ | `POST` | `/api/v1/show/query/config.json` | 0 | 66 | keys=['orderState', 'configOpens'] |
| ✅ | `POST` | `/api/v1/warehouse/getThirdWareCount.json` | 0 | 64 | int=0 |
| ✅ | `POST` | `/api_v2/api/v3/auth/loginsub.json` | -1 | 107 | null |

## Notes

- `POST /api_v2/api/v3/auth/loginsub.json` diuji **probe-only** (akun palsu); endpoint reachable + envelope JSON, bukan login sukses.
- `POST /api/v1/queryCondition/getOrderSearchTag.json` butuh body `{"module":"order","routeUrl":"/web/order/index.htm?status=new"}` (data bisa `null`).
- Core order: `pageList` total **49** new; `statusCountMap` new=49, shipped=403, completed=2745, canceled=587.
- CLI `orders list/counts/status` OK dengan session yang sama.

